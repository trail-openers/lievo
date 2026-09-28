//! Token-boundary query tokenizer and stop-word filter for `lievo_explore`
//! query mode (issue #837).
//!
//! Fixes the unanchored lowercase substring matcher: query words now match on
//! token boundaries rather than anywhere inside the lowercased name/path, so
//! "files" no longer admits "SettingsPanel" and "id" no longer admits
//! "validity". Prefix matching WITHIN a single token is preserved on
//! purpose — agents legitimately query partial identifiers and stems (e.g.
//! "auth" should still match "authentication") — while mid-token hits like
//! "id" in "validity" are rejected.
//!
//! Both call sites (admission in `tools_explore.rs` and scoring in
//! `explore_ranking.rs`) consume the same word set from `query_words`, so the
//! predicate lives here and is shared — admission and scoring cannot diverge.
//!
//! No stemming or NLP dependency: a hand-written, char-safe tokenizer (it
//! operates on `char`s, so non-ASCII identifiers cannot panic it) plus a small
//! explicit stop-word list.

/// Common English stop-words carried by natural-language agent queries (issue
/// #837). These words do not by themselves admit a file: a query made only of
/// them returns an empty word set and the matcher degrades gracefully. The
/// list is intentionally small and explicit (issue acceptance criterion:
/// "prefer a small explicit list over a large dependency"); each entry is a
/// word that, as an agent query, describes the act of looking rather than an
/// identifier.
const STOP_WORDS: &[&str] = &[
    "a",
    "an",
    "and",
    "any",
    "are",
    "as",
    "at",
    "be",
    "by",
    "find",
    "for",
    "from",
    "in",
    "into",
    "imports",
    "importing",
    "is",
    "it",
    "list",
    "on",
    "or",
    "the",
    "this",
    "that",
    "these",
    "those",
    "to",
    "using",
    "was",
    "were",
    "what",
    "when",
    "where",
    "which",
    "who",
    "why",
    "with",
];

/// True when `word` (lowercased) is one of the query stop-words.
pub(crate) fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.contains(&word)
}

/// Split `s` (expected already-lowercased) into boundary tokens: runs of
/// alphanumeric chars separated by everything else (`/ . _ -` and friends)
/// plus camelCase boundaries, so "orderExportApi" yields "order",
/// "export", "api". Alphanumeric (including unicode letters/digits, via
/// `char::is_alphanumeric`) stays inside a token; camelCase boundaries are
/// inserted between lowercase/digit -> uppercase transitions.
///
/// Char-safe by construction: the loop walks `char`s, so multi-byte
/// identifiers cannot panic or split mid-codepoint.
pub(crate) fn tokenize(s: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();

    let flush = |tokens: &mut Vec<String>, cur: &mut String| {
        if !cur.is_empty() {
            tokens.push(std::mem::take(cur));
        }
    };

    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_alphanumeric() {
            flush(&mut tokens, &mut cur);
            continue;
        }
        cur.push(c.to_ascii_lowercase());
        // camelCase boundary: current char is lowercase/digit and next is
        // uppercase — the current char is the last of the current token.
        if let Some(&next) = chars.peek()
            && next.is_ascii_uppercase()
            && (c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            flush(&mut tokens, &mut cur);
        }
    }
    flush(&mut tokens, &mut cur);

    tokens
}

/// True when `word` (lowercased) is a prefix of at least one token of `s`
/// (lowercased). The query word itself is also tokenized, so a separator
/// joined word (e.g. "selfcheck_metrics" or "app/foo/bar.js") is split into
/// its component tokens and EACH is checked independently as a token prefix
/// against the file's tokens. This preserves path-like query matching, stem
/// matching ("auth" -> "authentication"), and natural queries typed as the
/// filename itself ("selfcheck_metrics.rs" — the underscore is already a
/// token boundary, so each component token is tested on its own) while
/// eliminating cross-boundary and mid-token substring matches ("id" ⊄
/// "validity", "files" ⊄ "settingspanel").
pub(crate) fn word_matches(s: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let s_tokens = tokenize(s);
    let word_tokens = tokenize(word);
    // Every component token of the query word must independently be a prefix
    // of at least one file token. The single-token fast path is the same
    // rule with one element.
    s_tokens
        .iter()
        .any(|t| word_tokens.iter().any(|w| t.starts_with(w)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_camel_case() {
        let t = tokenize("orderExportApi");
        assert!(t.contains(&"order".to_string()));
        assert!(t.contains(&"export".to_string()));
        assert!(t.contains(&"api".to_string()));
    }

    #[test]
    fn tokenize_keeps_digit_runs() {
        let t = tokenize("v2api");
        assert_eq!(t, vec!["v2api".to_string()]);
    }

    #[test]
    fn tokenize_multibyte_does_not_panic() {
        let t = tokenize("日本語.rs/héllo");
        assert!(t.contains(&"rs".to_string()));
        assert!(t.contains(&"héllo".to_string()));
    }

    #[test]
    fn word_matches_prefix_within_token_not_across() {
        // Prefix matching is preserved: "auth" is a prefix of token
        // "authentication".
        assert!(word_matches("authentication", "auth"));
        // Mid-token hits are rejected: "id" is not a prefix of "validity".
        assert!(!word_matches("validity", "id"));
        assert!(!word_matches("middleware", "id"));
        // "files" is not a prefix of either token of "settings_panel"
        // (["settings", "panel"]).
        assert!(!word_matches("settings_panel", "files"));
        // "component" is a prefix of itself.
        assert!(word_matches("component", "component"));
        // "component" is not a prefix of "subcomponent".
        assert!(!word_matches("subcomponent", "component"));
        // Path-like multi-word queries use the same prefix rule.
        assert!(word_matches("orderExportApi/index.js", "order"));
        assert!(!word_matches("validity.js", "id"));
    }

    #[test]
    fn tokenize_underscores_and_dashes() {
        let tokens = tokenize("auth_service-util");
        assert!(tokens.contains(&"auth".to_string()));
        assert!(tokens.contains(&"service".to_string()));
        assert!(tokens.contains(&"util".to_string()));
    }

    #[test]
    fn stop_word_list_covers_issue_examples() {
        for w in ["the", "this", "that", "importing", "imports"] {
            assert!(is_stop_word(w), "{w} must be a stop word");
        }
        // Words that are legitimate identifier/path tokens must NOT be stop
        // words (issue #837: suppressing them broke honest queries like
        // "files", "component", "code").
        for w in [
            "auth",
            "token",
            "order",
            "files",
            "file",
            "code",
            "component",
            "function",
        ] {
            assert!(!is_stop_word(w), "{w} must NOT be a stop word");
        }
    }
}
