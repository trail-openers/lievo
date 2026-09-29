//! Per-file relevance scoring and width-bounded selection for `lievo_explore`
//! (issue #711).
//!
//! Extracted from `tools_explore.rs` to keep the main file under the 500-line
//! budget. The score channels are name + path only (no content channel, no
//! per-file I/O before the cap — PM decision 2026-09-12):
//!
//!   - a query word hit in the file NAME scores 2
//!   - a query word hit in the file PATH (and not the name) scores 1
//!
//! A word hitting both name and path counts once at the location max (2), so
//! no word double-counts — this holds even if `words` itself contains a
//! duplicate token (e.g. from a query like "auth auth"): `score_file_entity`
//! deduplicates its input before scoring, so "one distinct word" is enforced
//! regardless of how many times a word appears in the caller's word list.
//! Selection sorts by (score desc, path asc) — a stable, deterministic
//! tiebreak independent of storage order — and cuts to `max_files` BEFORE
//! symbol-building, so no I/O or relationship-scan work is spent on files
//! that will not be shown.

use crate::model::Entity;

/// Score one matched file entity against the (already filtered) query words.
/// Returns the numeric score and a short human reason naming which words hit
/// and where.
pub(crate) fn score_file_entity(entity: &Entity, words: &[String]) -> (i32, String) {
    let name = entity.name.to_lowercase();
    let path = entity.path.as_deref().unwrap_or("").to_lowercase();

    let mut score: i32 = 0;
    let mut name_hits: Vec<&String> = Vec::new();
    let mut path_hits: Vec<&String> = Vec::new();

    // Deduplicate `words` here too (first-occurrence order preserved) so a
    // repeated word can never double-count, regardless of whether the caller
    // already deduplicated (issue #711: "one distinct word counts once at
    // location max").
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let distinct_words = words.iter().filter(|w| seen.insert(w.as_str()));

    for w in distinct_words {
        let name_hit = crate::retrieval::query_tokenizer::word_matches(&name, w);
        let path_hit = crate::retrieval::query_tokenizer::word_matches(&path, w);
        if name_hit {
            score += 2;
            name_hits.push(w);
        } else if path_hit {
            score += 1;
            path_hits.push(w);
        }
    }

    // NOTE: a word hitting BOTH name and path counts once at the location max
    // (name = 2), not 2 + 1 = 3. The `else if` above enforces this: a word
    // that hits the name is not also counted as a path hit. This is the
    // "one distinct word, location max" rule from the issue's edge cases.

    let mut parts: Vec<String> = Vec::new();
    if !name_hits.is_empty() {
        parts.push(format!(
            "name: {}",
            name_hits
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !path_hits.is_empty() {
        parts.push(format!(
            "path: {}",
            path_hits
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let reason = if parts.is_empty() {
        String::from("no query-word hit")
    } else {
        parts.join("; ")
    };

    (score, reason)
}

/// Deterministically order matched file entities by score desc with a stable
/// path-based tiebreak (path asc), then cut to `max_files`. Entities that
/// clear the cut carry `(entity, score, reason)`; `not_shown` is the number
/// of matched entities cut (0 when the cap did not bind).
/// Rank + cut with an optional symbol-channel score: each entity's final
/// score is `max(score_file_entity(entity, words), symbol_score.get(id))`.
/// An exact symbol-name hit (4) therefore outranks a file name hit (2) and a
/// path token (1); a symbol prefix (2) outranks a path token (1). The reason
/// string gains a `symbol: …` clause when the symbol channel raised the
/// score. `symbol_score` empty (no symbol confirmed) behaves byte-identically
/// to the pre-symbol behaviour — existing ranking tests keep passing through
/// this path.
pub(crate) fn rank_and_cut_with_symbols(
    entities: Vec<Entity>,
    words: &[String],
    max_files: usize,
    symbol_score: &std::collections::HashMap<String, i32>,
) -> (Vec<(Entity, i32, String)>, usize) {
    let mut scored: Vec<(Entity, i32, String)> = entities
        .into_iter()
        .map(|e| {
            let (mut s, mut r) = score_file_entity(&e, words);
            if let Some(sym) = symbol_score.get(&e.id)
                && *sym > s
            {
                s = *sym;
                r.push_str(
                    if *sym >= crate::retrieval::tools_explore_symbols::EXACT_SYMBOL_SCORE {
                        "; symbol: exact name match"
                    } else {
                        "; symbol: name prefix match"
                    },
                );
            }
            (e, s, r)
        })
        .collect();

    scored.sort_by(|a, b| {
        let pa = a.0.path.as_deref().unwrap_or("");
        let pb = b.0.path.as_deref().unwrap_or("");
        b.1.cmp(&a.1).then_with(|| pa.cmp(pb))
    });

    let total = scored.len();
    let not_shown = total.saturating_sub(max_files.min(total));
    scored.truncate(max_files);

    (scored, not_shown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EntityTier;

    fn entity(id: &str, name: &str, path: Option<&str>) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: "p".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: name.to_string(),
            path: path.map(|s| s.to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn score_name_hit_outranks_path_only_hit() {
        let words = vec!["alpha".to_string()];
        let name_hit = entity("a", "alpha_util", Some("misc.rs"));
        let path_hit = entity("b", "beta", Some("deep/alpha/dir.rs"));
        let (s1, r1) = score_file_entity(&name_hit, &words);
        let (s2, r2) = score_file_entity(&path_hit, &words);
        assert!(s1 > s2, "name hit ({s1}) must outrank path hit ({s2})");
        assert!(r1.contains("name: alpha"));
        assert!(r2.contains("path: alpha"));
    }

    #[test]
    fn word_hitting_name_and_path_counts_once_at_max() {
        let words = vec!["auth".to_string()];
        let both = entity("a", "auth", Some("src/auth.rs"));
        let (s, r) = score_file_entity(&both, &words);
        assert_eq!(s, 2, "one distinct word, location max, no double count");
        assert_eq!(r, "name: auth");
    }

    #[test]
    fn repeated_word_in_query_counts_once_not_per_occurrence() {
        // A caller that hands score_file_entity a `words` slice with a
        // duplicate token (as an un-deduplicated query like "auth auth"
        // would naively produce) must still score it as one distinct word.
        // query_words() is the real dedup point; this test locks the
        // scoring-side contract regardless of how `words` was built.
        let words = vec!["auth".to_string(), "auth".to_string()];
        let name_hit = entity("a", "auth_service", Some("misc.rs"));
        let (s, r) = score_file_entity(&name_hit, &words);
        assert_eq!(s, 2, "a repeated word must not inflate the score");
        assert_eq!(r, "name: auth", "reason must not list a word twice");
    }

    #[test]
    fn multiple_words_accumulate() {
        let words = vec!["token".to_string(), "auth".to_string(), "zzz".to_string()];
        let multi = entity("a", "auth_token", Some("src/zzz.rs"));
        let (s, r) = score_file_entity(&multi, &words);
        assert_eq!(s, 5, "name hits: token+auth = 4, path hit: zzz = 1");
        assert!(r.contains("name: token, auth"));
        assert!(r.contains("path: zzz"));
    }

    #[test]
    fn rank_and_cut_sorts_score_desc_path_asc_and_reports_not_shown() {
        let words = vec!["def".to_string()];
        let mk = |n: &str| entity(n, n, Some(&format!("files/{n}.rs")));
        // def_beta and def_alpha: name hits (2 each), tie → path asc.
        // gamma in a path dir: path hit (1).
        let ents = vec![mk("def_beta"), mk("gamma"), mk("def_alpha")];
        let empty = std::collections::HashMap::new();
        let (kept, not_shown) = rank_and_cut_with_symbols(ents, &words, 2, &empty);
        let names: Vec<String> = kept.iter().map(|(e, _, _)| e.name.clone()).collect();
        assert_eq!(names, vec!["def_alpha", "def_beta"]);
        assert_eq!(not_shown, 1);
    }

    #[test]
    fn rank_and_cut_no_cut_when_max_exceeds_total() {
        let words = vec!["def".to_string()];
        let mk = |n: &str| entity(n, n, Some(&format!("files/{n}.rs")));
        let ents = vec![mk("def_a"), mk("def_b")];
        let empty = std::collections::HashMap::new();
        let (kept, not_shown) = rank_and_cut_with_symbols(ents, &words, 8, &empty);
        assert_eq!(kept.len(), 2);
        assert_eq!(not_shown, 0);
    }
}
