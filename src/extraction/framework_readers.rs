// Manifest reader helpers — one pure extractor per ecosystem.
// All I/O goes through `read_file_limited` to guard against huge/binary files.

use std::fs;
use std::path::Path;

use crate::error::Result;

/// 4 MB cap: manifests are text files; anything larger is suspicious.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

/// Read a file only if it exists and is ≤ `MAX_MANIFEST_BYTES`.
/// Returns `Ok(None)` for missing, oversized, or non-UTF-8 files.
pub(crate) fn read_file_limited(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let size = fs::metadata(path)?.len();
    if size > MAX_MANIFEST_BYTES {
        return Ok(None);
    }
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(_) => Ok(None), // binary or encoding error
    }
}

// ── Gemfile ────────────────────────────────────────────────────────────────

pub(crate) fn extract_gemfile_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("gem ") {
            let rest = rest.trim();
            if let Some(name) = extract_quoted_word(rest)
                && !name.is_empty()
            {
                deps.push(name.to_string());
            }
        }
    }
    deps
}

// ── requirements.txt ──────────────────────────────────────────────────────

pub(crate) fn extract_requirements_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }
        let name = trimmed
            .split(['=', '>', '<', '~', '[', ';'])
            .next()
            .unwrap_or(trimmed)
            .trim();
        if !name.is_empty() {
            deps.push(name.to_string());
        }
    }
    deps
}

// ── pyproject.toml ────────────────────────────────────────────────────────

pub(crate) fn extract_pyproject_deps(value: &toml::Value) -> Vec<String> {
    let mut deps = Vec::new();

    // [project.dependencies] — PEP 508 strings like "requests>=2.0"
    if let Some(arr) = value
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for item in arr {
            if let Some(s) = item.as_str() {
                let name = s
                    .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() {
                    deps.push(name.to_string());
                }
            }
        }
    }

    // [tool.poetry.dependencies] — table; keys are package names (skip "python")
    if let Some(table) = value
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for key in table.keys() {
            if key != "python" && !key.is_empty() {
                deps.push(key.clone());
            }
        }
    }

    deps.sort();
    deps.dedup();
    deps
}

// ── go.mod ────────────────────────────────────────────────────────────────

pub(crate) fn extract_go_mod_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_require = false;
    for line in content.lines() {
        let trimmed = line.trim();
        // Handle both `require (` and `require(`
        if !in_require
            && trimmed.starts_with("require")
            && trimmed.contains('(')
            && !trimmed.contains(')')
        {
            in_require = true;
            continue;
        }
        if in_require && trimmed == ")" {
            in_require = false;
            continue;
        }
        // Single-line `require foo/bar v1.0`
        if let Some(rest) = trimmed.strip_prefix("require ") {
            let rest = rest.trim();
            // Skip block-open `require (`
            if rest == "(" {
                in_require = true;
                continue;
            }
            let pkg = rest.split_whitespace().next().unwrap_or("").to_string();
            if !pkg.is_empty() {
                deps.push(pkg);
            }
            continue;
        }
        if in_require && !trimmed.is_empty() && !trimmed.starts_with("//") {
            let pkg = trimmed.split_whitespace().next().unwrap_or("").to_string();
            if !pkg.is_empty() {
                deps.push(pkg);
            }
        }
    }
    deps
}

// ── pom.xml ───────────────────────────────────────────────────────────────

/// Extract `<artifactId>` values only from inside `<dependency>` blocks.
pub(crate) fn extract_pom_artifact_ids(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_dependency = false;
    let mut search = content;

    loop {
        // Find the next interesting tag
        let dep_open = search.find("<dependency>");
        let dep_close = search.find("</dependency>");
        let artifact = search.find("<artifactId>");

        match (dep_open, dep_close, artifact) {
            // No more tags
            (None, None, None) => break,
            (None, None, Some(_)) => break,
            // Enter dependency block
            (Some(d), _, Some(a)) if !in_dependency && d < a => {
                search = &search[d + "<dependency>".len()..];
                in_dependency = true;
            }
            (Some(d), _, None) if !in_dependency => {
                search = &search[d + "<dependency>".len()..];
                in_dependency = true;
            }
            // Exit dependency block
            (_, Some(c), _) if in_dependency => {
                if let Some(a) = artifact {
                    if a < c {
                        // artifactId before closing tag — capture it
                        let tag_open = "<artifactId>";
                        let tag_close = "</artifactId>";
                        if let Some(start) = search.find(tag_open) {
                            let after_open = &search[start + tag_open.len()..];
                            if let Some(end) = after_open.find(tag_close) {
                                let name = after_open[..end].trim().to_string();
                                if !name.is_empty() {
                                    deps.push(name);
                                }
                            }
                            // Advance past closing tag, not opening, to process additional artifacts
                            if let Some(close_pos) = search[start..].find(tag_close) {
                                search = &search[start + close_pos + tag_close.len()..];
                            } else {
                                // No closing tag found, exit dependency block
                                search = &search[c + "</dependency>".len()..];
                                in_dependency = false;
                            }
                        } else {
                            // Tag not found (unexpected), exit dependency block
                            search = &search[c + "</dependency>".len()..];
                            in_dependency = false;
                        }
                    } else {
                        search = &search[c + "</dependency>".len()..];
                        in_dependency = false;
                    }
                } else {
                    search = &search[c + "</dependency>".len()..];
                    in_dependency = false;
                }
            }
            // artifactId outside dependency block — skip it
            (_, _, Some(a)) if !in_dependency => {
                search = &search[a + "<artifactId>".len()..];
            }
            // Dependency open with no artifact yet
            (Some(d), _, _) if !in_dependency => {
                search = &search[d + "<dependency>".len()..];
                in_dependency = true;
            }
            // Fallback: skip one character to avoid infinite loop
            _ => {
                if search.is_empty() {
                    break;
                }
                search = &search[1..];
            }
        }
    }
    deps
}

// ── build.gradle ──────────────────────────────────────────────────────────

pub(crate) fn extract_gradle_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("implementation") {
            let rest = rest.trim().trim_start_matches('(').trim();
            if let Some(quoted) = extract_quoted_word(rest) {
                let artifact = quoted.split(':').nth(1).unwrap_or(quoted).to_string();
                if !artifact.is_empty() {
                    deps.push(artifact);
                }
            }
        }
    }
    deps
}

// ── mix.exs ───────────────────────────────────────────────────────────────

/// Extract dependency atom names from `defp deps` function body only.
pub(crate) fn extract_mix_deps(content: &str) -> Vec<String> {
    // Find `defp deps` and collect text until the bare `end` that closes it.
    let marker = "defp deps";
    let Some(start) = content.find(marker) else {
        return vec![];
    };
    let body_start = start + marker.len();

    // Walk forward to find the `end` that closes `defp deps`.
    // depth starts at 1 because `defp deps do` already opened the block.
    // Skip the opening `do` on that same line so it isn't double-counted.
    let body = &content[body_start..];
    // Skip past the `do` that opens `defp deps do`, then start counting.
    let scan_start = body.find("do").map(|pos| pos + 2).unwrap_or(0);
    let mut depth: i32 = 1;
    let mut end_pos = body.len();
    let mut i = scan_start;
    while i < body.len() {
        let remaining = &body[i..];
        if remaining.starts_with("do")
            && (i == 0 || !body[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        {
            let after = &remaining[2..];
            if after.is_empty() || !after.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                depth += 1;
                i += 2;
                continue;
            }
        }
        if let Some(after) = remaining.strip_prefix("end") {
            let before_ok =
                i == 0 || !body[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_');
            let after_ok =
                after.is_empty() || !after.starts_with(|c: char| c.is_alphanumeric() || c == '_');
            if before_ok && after_ok {
                depth -= 1;
                if depth <= 0 {
                    end_pos = i;
                    break;
                }
            }
        }
        i += 1;
    }

    let scope = &body[..end_pos];

    // Extract {:name, patterns within that scope only
    let mut deps = Vec::new();
    let needle = "{:";
    let mut search = scope;
    while let Some(pos) = search.find(needle) {
        search = &search[pos + needle.len()..];
        let name: String = search
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            deps.push(name);
        }
    }

    deps.sort();
    deps.dedup();
    deps
}

// ── *.csproj ──────────────────────────────────────────────────────────────

pub(crate) fn extract_csproj_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let needle = "<PackageReference Include=\"";
    let mut search = content;
    while let Some(start) = search.find(needle) {
        search = &search[start + needle.len()..];
        if let Some(end) = search.find('"') {
            let name = search[..end].trim().to_string();
            if !name.is_empty() {
                deps.push(name);
            }
            search = &search[end + 1..];
        } else {
            break;
        }
    }
    deps
}

// ── Cargo.toml ────────────────────────────────────────────────────────────

/// Extract dependency names from `[dependencies]` and `[dev-dependencies]`
/// sections of a Cargo.toml. Keys are exact crate names.
pub(crate) fn extract_cargo_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let value: toml::Value = match toml::from_str(content) {
        Ok(v) => v,
        Err(_) => return deps,
    };
    for section in &["dependencies", "dev-dependencies"] {
        if let Some(table) = value.get(section).and_then(|v| v.as_table()) {
            for key in table.keys() {
                deps.push(key.clone());
            }
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

// ── Package.swift ─────────────────────────────────────────────────────────

/// Extract package names from `.package(url:...)` lines.
/// Tries `name:` parameter first, then falls back to URL last path segment.
pub(crate) fn extract_swift_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.contains(".package(") {
            continue;
        }
        // Try `name: "foo"` first
        if let Some(name) = extract_swift_name_param(trimmed) {
            deps.push(name);
        } else if let Some(name) = extract_swift_url_basename(trimmed) {
            deps.push(name);
        }
    }
    deps
}

/// Extract from `name: "value"` in a Swift .package() call.
fn extract_swift_name_param(s: &str) -> Option<String> {
    let needle = "name:";
    let pos = s.find(needle)?;
    let rest = s[pos + needle.len()..].trim();
    let name = extract_quoted_word(rest)?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Extract the last path segment from a URL, stripping `.git` suffix.
fn extract_swift_url_basename(s: &str) -> Option<String> {
    // Find url: "..." or url: '...'
    let needle = "url:";
    let pos = s.find(needle)?;
    let rest = s[pos + needle.len()..].trim();
    let url = extract_quoted_word(rest)?;
    // Take last non-empty path segment
    let basename = url
        .trim_end_matches('/')
        .rsplit('/')
        .find(|seg| !seg.is_empty())?;
    let name = basename.strip_suffix(".git").unwrap_or(basename);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

// ── Shared utility ─────────────────────────────────────────────────────────

/// Extract the first single- or double-quoted word from a string.
pub(crate) fn extract_quoted_word(s: &str) -> Option<&str> {
    let quote = if s.starts_with('\'') {
        '\''
    } else if s.starts_with('"') {
        '"'
    } else {
        return None;
    };
    let inner = &s[1..];
    let end = inner.find(quote)?;
    Some(&inner[..end])
}

#[cfg(test)]
#[path = "framework_readers_tests.rs"]
mod framework_readers_tests;
