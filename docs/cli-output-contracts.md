# CLI Output Contracts

Lievo CLI output follows strict contracts for human and JSON formats to enable reliable scriptability and agent integration.

## Format Enum

Two output formats are supported:

```rust
pub enum OutputFormat {
    Human, // Default
    Json,  // NDJSON
}
```

**Why two formats**: Human format is optimized for readability with aligned columns. JSON format is optimized for machine parsing and streaming. No additional formats are planned.

## Contract: Human Format

Human format uses aligned columns with headers for tabular data:

- Column alignment for numeric/ID fields
- Truncation of long text (paths, summaries) at column boundaries
- Newline-separated rows
- Header row present on each output

**Rationale**: Human format is for interactive use. Layout stability is not guaranteed — scripts should use JSON format instead.

Example pattern:
```
ID           Name          Tier        Files
------------------------------------------------
mod-123      parser        Module      15
mod-456      analyzer      Module      8
```

## Contract: JSON Format (NDJSON)

JSON format uses NDJSON (Newline-Delimited JSON). Each result is a valid JSON object on its own line:

1. **One JSON object per line** — No arrays wrapping results
2. **No trailing commas** — Standard JSON serialization
3. **Compact serialization** — No pretty-printing or extra whitespace
4. **Valid JSON per line** — Tools like `jq` can process line-by-line

Example pattern:
```json
{"id":"mod-123","name":"parser","tier":"Module","files":15}
{"id":"mod-456","name":"analyzer","tier":"Module","files":8}
```

**Why NDJSON over JSON array**: NDJSON supports streaming output. Results can be generated and sent to pipes incrementally without buffering the entire result set in memory. Large query results become feasible without requiring the CLI to hold the entire response in a Vec.

**Error handling in JSON mode**: Errors are always emitted to stderr (never mixed into the JSON stream). This keeps stdout pure JSON for parsers.

## Serialization Approach

All JSON output uses `serde_json`'s compact serialization:

```rust
let json = serde_json::to_string(&data)?;
println!("{json}");
```

**Why not arrays**: Arrays require the CLI to:
1. Fetch all results
2. Buffer into a collection
3. Serialize to JSON

NDJSON allows:
1. Fetch and serialize one result
2. Write immediately to stdout
3. Repeat

For queries returning thousands of entities, this difference determines whether memory usage is bounded or grows linearly with result size.

## Field Stability

JSON field names and structure are part of the public API contract:

- Field names follow `snake_case` (aligned with Rust structs)
- Required fields are always present
- Optional fields are omitted when `None` rather than using `null` in most cases

**Rationale**: Snake_case matches idiomatic Rust JSON serialization patterns. Optional fields are omitted to reduce payload size and clarify missing vs present-but-null semantics.

## Contract for Specific Commands

Different commands emit different JSON structures, but all follow the NDJSON pattern:

### Entity listings
```json
{"id":"...", "name":"...", "tier":"...", "files":0, "complexity":0.0}
```

### Entity details
```json
{"id":"...", "name":"...", "summary":"...", "metrics":{...}}
```

### Relationships
```json
{"source_id":"...", "target_id":"...", "type":"..."}
```

See individual formatter modules (`src/output/*.rs`) for the exact contracts per command.

## Backward Compatibility

Field additions are allowed within contracts. Field removal or renames are considered breaking changes and must follow semantic versioning.

**Principle**: Additive-only changes preserve existing parsers. Clients should ignore unknown fields rather than fail on them.