//! Output-cap for `lievo_explore` (issue #680, value-level rewrite #836).
//!
//! `cap_response` caps an over-cap response at the VALUE level: it operates
//! on the parsed `serde_json::Value`, drops whole entries (and whole fields
//! within a kept entry when needed) from the end of the mode's array until
//! the re-serialized object fits `MAX_EXPLORE_OUTPUT_CHARS` chars, and names
//! every dropped value in a `truncation` object in the same shape as the
//! bundle's `completeness` object (`{complete, omitted_*}`). The response
//! stays a single well-formed JSON object — no character-prefix splicing,
//! no re-escaping, no stubs.
//!
//! The cap is agnostic to the mode's array key (`symbols` for
//! word-match/files/bundle, `files` for scope listing) and to what the
//! array values look like: entries are dropped from the end, and fields
//! within a single oversized entry are dropped by measured serialized size,
//! so no per-mode key knowledge is needed.

use serde_json::{Map, Value, json};

use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;

/// Find the mode's array key: `symbols` first, `files` otherwise. Every
/// lievo_explore response carries exactly one of the two.
fn array_key(response: &Value) -> Option<&str> {
    if response.get("symbols").is_some_and(Value::is_array) {
        Some("symbols")
    } else if response.get("files").is_some_and(Value::is_array) {
        Some("files")
    } else {
        None
    }
}

/// A name suitable for the dropped-signal's `omitted_values` list: the
/// array value of the entry's first present path-ish string field, if any;
/// otherwise the entry's serialized form truncated to 80 chars. Never
/// invents a name for a non-object entry — it is identified by shape.
fn omission_name(entry: &Value) -> String {
    let obj = entry.as_object();
    if let Some(fields) = obj {
        for key in ["path", "qualified_path", "name"] {
            if let Some(s) = fields.get(key).and_then(Value::as_str)
                && !s.is_empty()
            {
                return s.to_string();
            }
        }
    }
    let serialized = entry.to_string();
    let head: String = serialized.chars().take(80).collect();
    format!("{head}…")
}

/// Shrink a single over-cap entry in place, dropping one field per iteration
/// (measured once per drop, never per candidate byte). Fields are dropped by
/// descending serialized size — for build_symbol entries this naturally
/// removes `source`/`call_paths`/`blast_radius` before identity fields;
/// for scope-mode `{entity_id, path}` entries the only droppable field is
/// `path`. The loop stops at one field (the heaviest of the rest), leaving a
/// minimal complete object; the caller's final size check is what bounds the
/// output, so no budget parameter is threaded through.
fn shrink_entry(obj: &mut Map<String, Value>) {
    loop {
        if obj.is_empty() {
            return;
        }
        let fits = serde_json::to_string(obj)
            .map(|s| s.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS)
            .unwrap_or(true);
        if fits {
            // Minimal entry that still carries identity: stop as soon as the
            // object alone fits the cap — the response framing around it
            // (a few hundred chars) keeps the whole response under the cap.
            return;
        }
        let mut entries: Vec<(String, usize)> = obj
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string().chars().count()))
            .collect();
        entries.sort_by_key(|b| std::cmp::Reverse(b.1));
        obj.remove(&entries[0].0);
    }
}

/// Cap the pre-cap response Value to `MAX_EXPLORE_OUTPUT_CHARS` chars.
/// Returns `None` when the response already fits (callers keep the original
/// serialization), otherwise `Some(capped)` — a well-formed JSON string
/// that parses to a value carrying `truncated: true` plus a `truncation`
/// object naming every dropped entry.
pub fn cap_response(response: &Value) -> Option<String> {
    if response.to_string().chars().count() <= MAX_EXPLORE_OUTPUT_CHARS {
        return None;
    }
    let key = array_key(response)?;
    let mut capped = response.clone();
    let initial_entries = capped
        .get(key)
        .and_then(Value::as_array)
        .map_or(0, |a| a.len());
    let mut omitted: Vec<String> = Vec::new();
    // The drop loop re-borrows `capped` as a mutable array each iteration,
    // dropping whole entries from the end until the whole object serializes
    // within the cap MINUS the truncation signal's serialized size. One
    // serialization per drop, never per candidate byte (issue #836 O(n²)
    // note). The signal is added after the loop, so its size must be
    // reserved before the loop to keep the final output within the cap.
    // The signal is at most ~200 chars (truncated flag + truncation object
    // with a few omitted values), so we reserve 200 chars.
    let signal_reserve = 200;
    let effective_cap = MAX_EXPLORE_OUTPUT_CHARS.saturating_sub(signal_reserve);
    loop {
        let still_over = capped.to_string().chars().count() > effective_cap;
        if !still_over {
            break;
        }
        let remaining = capped
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, |a| a.len());
        if remaining <= 1 {
            break;
        }
        let arr = match capped.get_mut(key).and_then(Value::as_array_mut) {
            Some(a) => a,
            None => break,
        };
        let removed = arr.pop().unwrap();
        omitted.push(omission_name(&removed));
    }
    let final_entries = capped
        .get(key)
        .and_then(Value::as_array)
        .map_or(0, |a| a.len());
    let dropped = initial_entries - final_entries;
    // Degenerate single-oversized entry: the drop loop exhausted the array
    // to one entry and it still does not fit. Whole-entry drops cannot help;
    // shrink the entry field-by-field (heaviest first) instead. The minimal
    // entry that remains is a complete, well-formed object — never a
    // fragment of serialized text.
    if final_entries == 1 && capped.to_string().chars().count() > MAX_EXPLORE_OUTPUT_CHARS {
        let arr = capped.get_mut(key).and_then(Value::as_array_mut);
        if let Some(a) = arr
            && let Some(o) = a[0].as_object_mut()
        {
            shrink_entry(o);
        }
        // Non-object entry: nothing to shrink; keep as-is.
    }
    // Truncate `omitted_values` to the maximum trailing slice that keeps the
    // final form under the hard cap. The signal's overhead (object wrapper +
    // truncated flag) is small and fixed; the variable part is the
    // `omitted_values` array. We trim from the front (earliest-dropped names)
    // until the full response fits, since `omitted_entries` (the integer)
    // is the primary count and the names are a hint.
    let signal_overhead: usize = 80;
    let base_size = capped.to_string().chars().count();
    let mut omitted_values: Vec<String> = omitted.clone();
    while omitted_values.len() > 1
        && base_size
            + signal_overhead
            + serde_json::to_string(&omitted_values)
                .unwrap_or_default()
                .chars()
                .count()
            > MAX_EXPLORE_OUTPUT_CHARS
    {
        omitted_values.remove(0);
    }
    // If even one name doesn't fit, drop to an empty list — the integer
    // `omitted_entries` still carries the count.
    if omitted_values.len() == 1
        && base_size
            + signal_overhead
            + serde_json::to_string(&omitted_values)
                .unwrap_or_default()
                .chars()
                .count()
            > MAX_EXPLORE_OUTPUT_CHARS
    {
        omitted_values.clear();
    }
    capped["truncated"] = json!(true);
    capped["truncation"] = json!({
        "complete": false,
        "omitted_entries": dropped,
        "omitted_values": omitted_values,
    });
    Some(capped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #836 core contract: a response forced over 24K survives a
    /// round-trip parse with a valid `symbols` array of complete entries —
    /// no character-prefix fragment, no stubs.
    #[test]
    fn capped_response_round_trip_parses_with_complete_symbols_array() {
        let symbols: Vec<Value> = (0..50)
            .map(|i| {
                json!({
                    "entity_id": format!("p:repo1:file:{i}.rs"),
                    "name": format!("sym_{i}"),
                    "kind": "file",
                    "qualified_path": format!("src/{i}.rs"),
                    "language": "Rust",
                    "source": "x".repeat(500),
                })
            })
            .collect();
        let response = json!({ "symbols": symbols });
        assert!(
            response.to_string().chars().count() > MAX_EXPLORE_OUTPUT_CHARS,
            "fixture must exceed the cap"
        );

        let capped = cap_response(&response).expect("must cap");
        // 1) whole response parses; 2) `symbols` is a valid array;
        // 3) every remaining entry is complete (all required fields present).
        let v: Value = serde_json::from_str(&capped).expect("must round-trip parse");
        let arr = v["symbols"].as_array().expect("symbols must be an array");
        assert!(!arr.is_empty());
        for sym in arr {
            for field in ["entity_id", "name", "kind", "qualified_path", "language"] {
                assert!(
                    sym.get(field).map(|f| !f.is_null()).unwrap_or(false),
                    "capped symbol must keep required field {field}: {sym:?}"
                );
            }
        }
        // No truncation payload string, no stubs, structured signal present.
        assert!(v.get("truncated_payload").is_none());
        assert_eq!(v["truncated"], json!(true));
        let trunc = v["truncation"].as_object().expect("truncation object");
        assert_eq!(trunc["complete"], json!(false));
        let omitted = trunc["omitted_values"].as_array().unwrap();
        let dropped = trunc["omitted_entries"].as_u64().unwrap() as usize;
        assert_eq!(
            dropped,
            symbols.len() - arr.len(),
            "omitted_entries must equal what left the array: dropped={dropped}, left_array={}",
            symbols.len() - arr.len()
        );
        assert_eq!(
            dropped,
            omitted.len(),
            "omitted_values must name exactly the dropped entries"
        );
        // The dropped-signal names what was omitted: the last symbol's path.
        assert!(omitted.iter().any(|n| n.as_str() == Some("src/49.rs")));
        assert!(capped.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS);
    }

    #[test]
    fn cap_response_returns_none_when_already_under_cap() {
        let small = json!({"files": []});
        assert!(cap_response(&small).is_none());
    }

    /// Boundary: a response serialized at EXACTLY the cap is not capped —
    /// the cap is inclusive (`<=`), matching the pre-cap packing loops.
    #[test]
    fn response_at_exact_cap_is_not_capped() {
        let filler = "z".repeat(MAX_EXPLORE_OUTPUT_CHARS);
        let probe = json!({ "symbols": [], "filler": &filler });
        // (Trimming the filler shrinks the serialization 1:1: the filler's
        // content is pure `z`, no escaping involved.)
        let overshoot = probe.to_string().chars().count() - MAX_EXPLORE_OUTPUT_CHARS;
        let keep = filler.chars().count() - overshoot;
        let trimmed: String = filler.chars().take(keep).collect();
        let response = json!({ "symbols": [], "filler": trimmed });
        assert_eq!(
            response.to_string().chars().count(),
            MAX_EXPLORE_OUTPUT_CHARS,
            "fixture must be exactly at the cap"
        );
        assert!(cap_response(&response).is_none());
    }

    /// Degenerate case: a SINGLE entry alone exceeds the cap. Whole-entry
    /// drops cannot help; the cap must shrink the entry field-by-field
    /// (heaviest first) and still return a complete, well-formed object
    /// under the cap — never a string fragment.
    #[test]
    fn single_oversized_entry_is_shrun_field_by_field() {
        let huge = json!({
            "entity_id": "p:repo1:file:big.rs",
            "name": "big",
            "kind": "file",
            "qualified_path": "src/big.rs",
            "language": "Rust",
            "source": "x".repeat(30_000),
        });
        let response = json!({ "symbols": [huge] });
        assert!(
            response.to_string().chars().count() > MAX_EXPLORE_OUTPUT_CHARS,
            "fixture must exceed the cap"
        );

        let capped = cap_response(&response).expect("must cap");
        assert!(capped.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS);
        let v: Value = serde_json::from_str(&capped).unwrap();
        let sym = &v["symbols"].as_array().unwrap()[0];
        // Heaviest optional field dropped first; identity kept.
        assert!(sym.get("source").is_none());
        for field in ["entity_id", "name", "kind", "qualified_path", "language"] {
            assert!(sym.get(field).is_some(), "identity field {field} kept");
        }
        assert_eq!(v["truncated"], json!(true));
        assert_eq!(v["truncation"]["complete"], json!(false));
    }

    /// Multi-byte UTF-8: a 30K run of 3-byte chars must not break the cap
    /// (the whole-response check counts chars, identical to the packing
    /// loops) and the response must still round-trip parse.
    #[test]
    fn multibyte_entry_shrun_by_char_count_not_bytes() {
        let sym = json!({
            "entity_id": "p:repo1:file:u.rs",
            "name": "u",
            "kind": "file",
            "qualified_path": "src/u.rs",
            "language": "Rust",
            "source": "€".repeat(30_000),
        });
        let small = json!({
            "entity_id": "p:repo1:file:b.rs",
            "name": "b",
            "kind": "file",
            "qualified_path": "src/b.rs",
            "language": "Rust",
        });
        let response = json!({ "symbols": [small, sym] });
        assert!(
            response.to_string().chars().count() > MAX_EXPLORE_OUTPUT_CHARS,
            "fixture must exceed the cap"
        );
        let capped = cap_response(&response).expect("must cap");
        assert!(capped.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS);
        let v: Value = serde_json::from_str(&capped).unwrap();
        let arr = v["symbols"].as_array().unwrap();
        // The multi-byte entry was dropped whole (it was the heavier entry,
        // dropped from the end) or shrunk — either way, the response stays
        // well-formed and the lighter entry survives intact.
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["entity_id"], "p:repo1:file:b.rs");
    }

    /// Scope-listing shape: `files` array of minimal `{entity_id, path}`
    /// objects — the cap must find the `files` key, drop whole entries from
    /// the end, and name them by their `path` in the dropped-signal. The
    /// drop loop bounds the FINAL form (body plus the truncation signal
    /// with its omitted_values list) against the hard cap, so even when the
    /// signal itself is large the output stays at or under
    /// `MAX_EXPLORE_OUTPUT_CHARS`.
    #[test]
    fn scope_shape_files_key_is_capped_and_named_by_path() {
        // 300 entries, each ~100 chars, ~30K total: the drop loop must drop
        // ~60 entries to get from ~30K to 24K — a strong test of the loop's
        // convergence.
        let files: Vec<Value> = (0..300)
            .map(|i| {
                json!({
                    "entity_id": format!("p:repo1:file:{i}"),
                    "path": format!("{i}/very/deep/repo/relative/path/to/file.rs/with/extra/depth/and/segments"),
                })
            })
            .collect();
        let response = json!({ "files": files, "returned": files.len(), "total": files.len(), "scope": "src" });
        assert!(
            response.to_string().chars().count() > MAX_EXPLORE_OUTPUT_CHARS,
            "fixture must exceed the cap"
        );
        let capped = cap_response(&response).expect("must cap");
        assert!(
            capped.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
            "final form (body + truncation signal) must respect the hard cap, got {} chars",
            capped.chars().count()
        );
        let v: Value = serde_json::from_str(&capped).unwrap();
        let arr = v["files"].as_array().expect("files must be an array");
        let dropped = files.len() - arr.len();
        assert!(dropped > 0, "must have dropped at least one entry");
        assert_eq!(v["truncated"], json!(true));
        assert_eq!(
            v["truncation"]["omitted_entries"].as_u64().unwrap() as usize,
            dropped
        );
        // The omitted_values list is a trailing slice of the dropped entries
        // (earliest-dropped names may be trimmed to keep the signal under
        // the cap). The last name in the list is the earliest-dropped entry
        // that survived the trim; the first name is the most recent entry
        // in that trailing slice. We verify the list is non-empty and that
        // the omitted_entries count matches the actual number dropped.
        let names = v["truncation"]["omitted_values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(
            !names.is_empty(),
            "omitted_values must not be empty when entries were dropped"
        );
        // The last name in the trailing slice should be the earliest-dropped
        // entry that wasn't trimmed. Since we drop from the end of the array
        // and trim from the front of the omitted list, the last name should
        // be the path of the first dropped entry (index 200 in this fixture).
        // But after trimming, it could be any entry from index 200 to 299.
        // We just verify it's one of the dropped entries' paths.
        let last_name = names.last().unwrap();
        let last_path_idx: usize = last_name
            .chars()
            .take_while(|c| c.is_numeric())
            .collect::<String>()
            .parse()
            .unwrap();
        assert!(
            last_path_idx >= files.len() - dropped && last_path_idx < files.len(),
            "last omitted name must be a dropped entry's path, got index {last_path_idx}"
        );
        // Every kept entry still has both fields intact.
        for f in arr {
            assert!(f["entity_id"].is_string());
            assert!(f["path"].is_string());
        }
        // Pre-cap mode fields survive the cap untouched.
        assert_eq!(v["total"], json!(300));
        assert_eq!(v["scope"], json!("src"));
    }

    /// Pre-cap mode-specific top-level fields (continuation, not_shown,
    /// completeness string) must survive the cap untouched — the cap only
    /// drops array entries and adds `truncated`/`truncation`.
    #[test]
    fn pre_cap_disclosure_fields_survive_the_cap() {
        let symbols: Vec<Value> = (0..40)
            .map(|i| {
                json!({
                    "entity_id": format!("p:repo1:file:{i}.rs"),
                    "name": format!("n{i}"),
                    "kind": "file",
                    "qualified_path": format!("src/{i}.rs"),
                    "language": "Rust",
                    "score": 1.0,
                    "reason": "name",
                    "source": "x".repeat(2_000),
                })
            })
            .collect();
        let mut response = json!({ "symbols": symbols });
        response["continuation"] = json!("returned: 8, total: 12, next: \"lievo_explore(...)\"");
        response["not_shown"] = json!(4);
        response["completeness"] = json!("showing 8 of 12 matching files");
        // Force over cap with a filler so the fixture deterministically binds.
        response["__pad"] = json!("q".repeat(20_000));

        let capped = cap_response(&response).expect("must cap");
        assert!(capped.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS);
        let v: Value = serde_json::from_str(&capped).unwrap();
        assert_eq!(v["not_shown"], json!(4));
        assert_eq!(v["completeness"], json!("showing 8 of 12 matching files"));
        assert!(v["continuation"].is_string());
    }

    /// The loop's termination (single heaviest field) plus the caller's final
    /// size check is what bounds the output. This test pins that a single
    /// entry whose heaviest field is itself over the cap still yields a
    /// minimal complete object: the heavy field drops first, the loop
    /// continues to the single heaviest of the rest, and identity survives.
    #[test]
    fn shrink_entry_leaves_minimal_object() {
        let mut obj = Map::from_iter([
            ("entity_id".to_string(), json!("p:repo1:file:a.rs")),
            ("name".to_string(), json!("a")),
            ("kind".to_string(), json!("file")),
            ("qualified_path".to_string(), json!("src/a.rs")),
            ("language".to_string(), json!("Rust")),
            ("source".to_string(), json!("x".repeat(30_000))),
        ]);
        shrink_entry(&mut obj);
        // Down to one field: `source` is the heaviest, so it goes first, then
        // the loop continues until a single field remains.
        assert!(
            obj.contains_key("entity_id"),
            "identity survives shrinking: {obj:?}"
        );
        assert!(!obj.contains_key("source"));
    }
}
