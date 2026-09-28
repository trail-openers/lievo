// Shared Rust `mod`-declaration parsing + test-file classification
// (issue #744).
//
// Both `analysis::module_map` (the `#[path]` alias-map walk) and the
// selfcheck's structural super:: probe (the selfcheck binary) need to
// parse semicolon `mod` declarations and exclude test-like files from
// module-tree reasoning. The implementations previously lived private in
// each place (`module_map::parse_mod_decls`,
// `selfcheck_false_zero::is_test_like_file`); this module is the single
// copy both sides call, so the two walkers cannot drift apart in how they
// read the same source.
//
// Deliberately dependency-free: std-only string parsing, no I/O — both
// callers supply file contents.

/// True when `file` is a test file that legitimately lacks recorded
/// importers / should be excluded from module-tree reasoning (test
/// harnesses reference test modules without an `Imports` edge). Excluded
/// from the false-0-callers scan (issue #715) and the structural
/// super:: probe's pre-read set (issue #744).
pub fn is_test_like_file(file: &str) -> bool {
    let path = std::path::Path::new(file);
    // (a) any path component is exactly "tests" (issue #725)
    // (a') any path component ends with "_tests" (issue #734 follow-up —
    // catches #[path]-declared module dirs like `convention_detector_tests/`,
    // `tools_search_validation_tests/`, which are test-only modules that
    // legitimately lack recorded importers without an Imports edge).
    let under_tests_dir = path
        .components()
        .any(|c| c.as_os_str() == "tests" || c.as_os_str().to_string_lossy().ends_with("_tests"));
    if under_tests_dir {
        return true;
    }
    let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    // (b) filename ends with a test suffix
    if file_name.ends_with("_tests.rs")
        || file_name.ends_with("_test.rs")
        || file_name.ends_with("tests.rs")
        || file_name.ends_with("test.rs")
    {
        return true;
    }
    // (b') filename contains "_tests" as a substring (issue #734 follow-up —
    // catches files like `schema_tests_migrations.rs`, `schema_tests_v7_v8.rs`,
    // which are test-harness files named after the module they test).
    if file_name.contains("_tests") {
        return true;
    }
    // (c) filename is exactly "lib.rs"
    file_name == "lib.rs"
}

/// (mod name, optional `#[path]` target string) pairs for every semicolon
/// `mod` declaration in `source`; see the `#[cfg(test)]` exclusion rules
/// and `#[path]` attribute-ordering semantics in the original
/// `module_map` module doc (issue #732). Inline `mod name { ... }` blocks
/// are not file references and are ignored.
pub fn parse_mod_decls(source: &str) -> Vec<(String, Option<String>)> {
    let mut pending: Option<String> = None;
    let mut pending_cfg_test = false;
    let mut decls: Vec<(String, Option<String>)> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[") {
            if trimmed.starts_with("#[cfg(test)]") {
                pending_cfg_test = true;
            } else if let Some(path) = parse_path_attr(trimmed) {
                pending = Some(path);
            }
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if let Some(name) = parse_mod_name(trimmed) {
            let cfg_test = pending_cfg_test;
            pending_cfg_test = false;
            let path_override = pending.take();
            // A `#[cfg(test)]`-gated declaration is not part of the non-test
            // module tree: its physical file is absent from non-test
            // extraction, and the walk must not descend into it (issue #732
            // round-2 finding #2). The pending `#[path]` value is consumed
            // (not leaked to the next declaration) but the declaration
            // itself is skipped.
            if cfg_test {
                continue;
            }
            decls.push((name, path_override));
        } else {
            // A non-`mod` line (e.g. the body of an inline `mod x { ... }`
            // block) resets the pending attribute state — without this,
            // `pending_cfg_test` set by a `#[cfg(test)]` attribute above an
            // inline module body would leak past the closing `}` and swallow
            // the next semicolon `mod` declaration (issue #863 regression:
            // the inline `test_env_support` module in `src/lib.rs` caused
            // the stale flag to drop `pub mod retrieval;` from the module
            // map).
            pending_cfg_test = false;
            pending = None;
        }
    }
    decls
}

/// The simple names the source text references as items — declaration
/// keywords (`fn`/`struct`/`enum`/`trait`/`type`/`const`/`static`/`union`),
/// `macro_rules! name`, and `use`/`pub use` paths (their last segment).
/// Line-based approximation for the super:: probe (issue #744 corrected
/// rule): it is NOT a parser — a mid-line use of a declaration keyword is
/// a deliberate false-positive trade, and the probe classifies only
/// CONFIRMED vs FAILURE (false confirms are safe; the failure bucket is
/// where misses would hide real defects). No I/O, no resolver, std-only.
pub fn find_item_names(source: &str) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(name) = item_name_from_line(trimmed) {
            names.insert(name);
        }
    }
    names
}

/// One line → the item name it declares or re-exports, or `None`. Only
/// the first declaration on a line is recognised (Rust convention: one
/// item per line; `rustfmt` normalises multi-item lines to one-per-line).
fn item_name_from_line(line: &str) -> Option<String> {
    // Declaration shape: `[pub(…)?] <kw> <name>`, with `<kw>` either at
    // the line start or preceded by a single space (so mid-line
    // occurrences in an expression, `foo::fn();`, are not matched).
    let vis = |rest: &str| matches!(rest, "" | "pub" | "pub(crate)" | "pub(super)" | "pub(self)");
    let tail = |line: &str, kw_end: usize| -> Option<String> {
        let after = line[kw_end..].trim_start();
        let name = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect::<String>();
        if ident_chars_only(&name) {
            Some(name)
        } else {
            None
        }
    };
    for kw in [
        "fn", "struct", "enum", "trait", "type", "const", "static", "union",
    ] {
        if line.starts_with(kw) && line.len() > kw.len() && line[kw.len()..].starts_with(' ') {
            return tail(line, kw.len());
        }
        let anchor = format!(" {kw} ");
        if let Some(pos) = line.rfind(&anchor) {
            let prefix = line[..pos].trim_end();
            if vis(prefix) || (prefix.ends_with("pub(") && prefix.contains(')')) {
                return tail(line, pos + 1 + kw.len());
            }
        }
    }
    if line.starts_with("macro_rules!")
        && line.len() > "macro_rules!".len()
        && line["macro_rules!".len()..].starts_with(' ')
    {
        return tail(line, "macro_rules!".len());
    }
    if let Some(pos) = line.rfind(" macro_rules!")
        && vis(line[..pos].trim_end())
    {
        return tail(line, pos + 1 + "macro_rules!".len());
    }
    // `use` / `pub use` paths: the last `::` segment names the item the
    // line makes available (a re-export ending in `X` counts as the
    // parent providing `X`; a direct `use` is not a provision, but
    // false-positives here only move sites failure → confirmed, which
    // is the safe direction for the probe).
    if let Some(rest) = line.strip_prefix("pub use ") {
        return use_tail_name(rest);
    }
    if let Some(rest) = line.strip_prefix("use ")
        && let Some(name) = use_tail_name(rest)
    {
        return Some(name);
    }
    None
}

/// The last `::` segment of a `use` path, when it is a plain identifier.
fn use_tail_name(rest: &str) -> Option<String> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let last = rest.rsplit_once("::").map(|(_, tail)| tail)?;
    let last = last.trim().trim_end_matches(';').trim();
    if ident_chars_only(last) {
        Some(last.to_string())
    } else {
        None
    }
}

/// Valid Rust identifier characters (alphanumeric + underscore, leading
/// char an alphabetic or underscore).
fn ident_chars_only(token: &str) -> bool {
    !token.is_empty()
        && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && matches!(
            token.chars().next(),
            Some(c) if c.is_ascii_alphabetic() || c == '_'
        )
}

/// `#[path = "..."]` on its own line → the quoted target.
fn parse_path_attr(line: &str) -> Option<String> {
    if !line.starts_with("#[") || !line.ends_with(']') {
        return None;
    }
    let inner = line[2..line.len() - 1].trim();
    let inner = inner.strip_prefix("path")?;
    let inner = inner.trim_start();
    let inner = inner.strip_prefix('=')?;
    let inner = inner.trim();
    let inner = inner
        .strip_prefix('"')
        .or_else(|| inner.strip_prefix('\''))?;
    let inner = inner
        .strip_suffix('"')
        .or_else(|| inner.strip_suffix('\''))?;
    Some(inner.to_string())
}

/// `mod name;`, `pub mod name;`, `pub(crate) mod name;` — semicolon
/// terminated only. Anchored to the declaration shape: the token BEFORE
/// `mod` must be nothing (bare `mod name;`) or a visibility prefix
/// (`pub` / `pub(crate)`), so a mid-line identifier followed by `mod x`
/// (e.g. an expression like `do_mod cleanup;`) cannot be mis-parsed as a
/// module declaration (issue #732 round-2 finding #3).
fn parse_mod_name(line: &str) -> Option<String> {
    let body = line.strip_suffix(';').unwrap_or(line);
    let name = body.rsplit(' ').next().unwrap_or("");
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    // Anchor: the declaration part is one of the three shapes, and nothing
    // other than the shown prefix may precede it on the line.
    if let Some(rest) = body.strip_suffix(&format!("mod {name}")) {
        let rest = rest.trim_end();
        return Some(match rest {
            "" | "pub" | "pub(crate)" => name.to_string(),
            _ => return None,
        });
    }
    None
}

#[cfg(test)]
#[path = "rust_mod_parse_tests.rs"]
mod rust_mod_parse_tests;
