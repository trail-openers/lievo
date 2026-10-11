//! Tests for repository identity derivation (see `identity.rs`).

use super::*;
use crate::identity_key::MAX_KEY_LEN;
use std::path::Path;
use tempfile::TempDir;

/// Table-driven cases: every URL form in the issue's edge-case list must
/// collapse to the same key `github.com/owner/repo`. Leading and trailing
/// whitespace (including a newline) is trimmed before the control-character
/// check.
#[test]
fn test_normalize_origin_url_all_spellings_collapse_to_one_key() {
    let expected = "github.com/owner/repo";

    let cases = [
        // scp-like SSH
        "git@github.com:owner/repo",
        "git@github.com:owner/repo.git",
        // ssh:// with explicit standard port and user
        "ssh://git@github.com:22/owner/repo",
        "ssh://git@github.com/owner/repo",
        "ssh://git@github.com/owner/repo.git",
        // HTTPS with and without credentials
        "https://github.com/owner/repo",
        "https://github.com/owner/repo.git",
        "https://github.com/owner/repo/",
        "https://user:token@github.com/owner/repo",
        "https://user@github.com/owner/repo.git",
        // git://
        "git://github.com/owner/repo",
        "git://github.com/owner/repo.git",
        "git://github.com:9418/owner/repo",
        // uppercase host must lowercase
        "https://GitHub.com/owner/repo",
        "git@GitHub.com:owner/repo",
        // trailing slash variants
        "ssh://git@github.com/owner/repo/",
        // trailing whitespace and newlines are trimmed
        "https://github.com/owner/repo ",
        "https://github.com/owner/repo\n",
    ];

    for url in &cases {
        let key = normalize_origin_url(url).unwrap_or_else(|| panic!("failed to normalize: {url}"));
        assert_eq!(
            key, expected,
            "URL {url} should normalize to {expected}, got {key}"
        );
    }
}

/// The derived key without its source, for comparing spellings.
fn key_of(identity: &Option<Identity>) -> Option<&str> {
    identity.as_ref().map(|i| i.key.as_str())
}

/// Build a tempdir git repo with an origin remote set to `url`.
fn make_repo_with_remote(dir: &Path, url: &str) -> git2::Repository {
    let repo = git2::Repository::init(dir).unwrap();
    if !url.is_empty() {
        repo.config()
            .unwrap()
            .set_str("remote.origin.url", url)
            .unwrap();
    }
    repo
}

fn default_config_with_identity(identity: &str) -> RepoConfig {
    RepoConfig {
        identity: Some(identity.to_string()),
        ..Default::default()
    }
}

#[test]
fn test_origin_and_override_mixed_case_collapse_to_same_key() {
    let tmp = TempDir::new().unwrap();
    let _ = make_repo_with_remote(tmp.path(), "git@github.com:Owner/Repo.git");
    let config = default_config_with_identity("github.com/owner/repo");
    let origin = derive_identity(tmp.path(), &RepoConfig::default());
    assert_eq!(
        key_of(&origin),
        key_of(&derive_identity(tmp.path(), &config))
    );
}

#[test]
fn test_normalize_origin_url_strips_credentials_and_token_never_leaks() {
    let url = "https://user:supersecrettoken123@github.com/owner/repo";
    let key = normalize_origin_url(url).unwrap();
    assert_eq!(key, "github.com/owner/repo");
    assert!(
        !key.contains("supersecrettoken123"),
        "token must not appear in the key: {key}"
    );
    assert!(
        !key.contains("user"),
        "user must not appear in the key: {key}"
    );
    assert!(!key.contains('@'), "no '@' in key: {key}");
}

#[test]
fn test_normalize_origin_url_explicit_standard_port_same_as_plain() {
    let a = normalize_origin_url("ssh://git@github.com:22/owner/repo").unwrap();
    let b = normalize_origin_url("git@github.com:owner/repo").unwrap();
    assert_eq!(a, b);
    assert_eq!(a, "github.com/owner/repo");
}

#[test]
fn test_normalize_origin_url_nonstandard_port_is_preserved() {
    let key = normalize_origin_url("ssh://git@github.com:2222/owner/repo").unwrap();
    assert_eq!(key, "github.com:2222/owner/repo");
    let key = normalize_origin_url("https://github.com:8443/owner/repo").unwrap();
    assert_eq!(key, "github.com:8443/owner/repo");
    // 9418 is standard for git://, but a different git:// port is kept.
    let key = normalize_origin_url("git://github.com:9419/owner/repo").unwrap();
    assert_eq!(key, "github.com:9419/owner/repo");
}

#[test]
fn test_normalize_origin_url_unparseable_returns_none() {
    for url in [
        "",
        "   ",
        "not-a-url",
        "https://",
        "http://github.com",
        "git@github.com",
        "ssh://",
        "ftp://github.com/owner/repo",
        "telnet://github.com/owner/repo",
    ] {
        assert!(
            normalize_origin_url(url).is_none(),
            "{url:?} must normalize to None"
        );
    }
}

#[test]
fn test_normalize_origin_url_requires_owner_and_repo() {
    // Host only, no path → None
    assert_eq!(normalize_origin_url("https://github.com/"), None);
    // A single-segment path is not owner/repo → None
    assert_eq!(normalize_origin_url("https://github.com/repo"), None);
    assert_eq!(normalize_origin_url("git@github.com:repo"), None);
}

#[test]
fn test_normalize_origin_url_rejects_dot_segments_and_invalid_ports() {
    // Path traversal / dot segments are never valid identity keys.
    for url in [
        "git@github.com:../../etc/passwd",
        "https://github.com/../evil/repo",
        "https://github.com/owner/.git",
        "ssh://github.com/owner/./repo",
        "git@github.com:owner//repo",
    ] {
        assert!(
            normalize_origin_url(url).is_none(),
            "{url:?} must normalize to None"
        );
    }
    // A port that is present but does not parse as u16 is a malformed
    // URL: the function returns None rather than silently dropping it.
    for url in [
        "ssh://github.com:65536/owner/repo",
        "https://github.com:abc/owner/repo",
    ] {
        assert!(
            normalize_origin_url(url).is_none(),
            "{url:?} must normalize to None"
        );
    }
}

#[test]
fn test_normalize_origin_url_contains_control_chars_returns_none() {
    // Control characters (ANSI escapes, newlines, NUL) in a remote URL
    // must never leak into an identity key.
    for url in [
        "git@github.com:owner/repo\u{001b}[31m",
        "https://github.com/owner/re\npo",
        "https://github.com/owner/\u{0}/repo",
        "ssh://github.com/owner\t/repo",
    ] {
        assert!(
            normalize_origin_url(url).is_none(),
            "URL with control characters must normalize to None"
        );
    }
}

#[test]
fn test_derive_identity_override_wins_without_any_git_repo() {
    // Override is checked before any git call: even a non-repo directory
    // with an override yields the override key.
    let tmp = TempDir::new().unwrap();
    let config = default_config_with_identity("Example.com/Some/Override");
    let identity = derive_identity(tmp.path(), &config);
    assert_eq!(key_of(&identity), Some("example.com/some/override"));
}

#[test]
fn test_derive_identity_override_wins_in_repo_without_remote() {
    // Repo exists but has no origin remote; override still wins (no error).
    let tmp = TempDir::new().unwrap();
    let _ = git2::Repository::init(tmp.path()).unwrap();
    let config = default_config_with_identity("example.com/other/repo");
    let identity = derive_identity(tmp.path(), &config);
    assert_eq!(key_of(&identity), Some("example.com/other/repo"));
}

#[test]
fn test_derive_identity_override_wins_over_live_remote() {
    let tmp = TempDir::new().unwrap();
    let _ = make_repo_with_remote(tmp.path(), "https://github.com/real/repo");
    let config = default_config_with_identity("example.com/override/repo");
    let identity = derive_identity(tmp.path(), &config);
    assert_eq!(key_of(&identity), Some("example.com/override/repo"));
}

#[test]
fn test_derive_identity_reports_the_source_of_the_key() {
    let tmp = TempDir::new().unwrap();
    let _ = make_repo_with_remote(tmp.path(), "https://github.com/real/repo");
    let from_override = derive_identity(tmp.path(), &default_config_with_identity("a.b/c/d"));
    assert_eq!(
        from_override.map(|i| i.source),
        Some(IdentitySource::Override)
    );
    let from_origin = derive_identity(tmp.path(), &RepoConfig::default());
    assert_eq!(from_origin.map(|i| i.source), Some(IdentitySource::Origin));
}

#[test]
fn test_derive_identity_override_with_dot_segment_returns_none() {
    // Defensive: an override that snuck past validation (e.g. written
    // directly) with a traversal segment must not become a key.
    let tmp = TempDir::new().unwrap();
    let config = default_config_with_identity("github.com/../evil/repo");
    let identity = derive_identity(tmp.path(), &config);
    assert!(identity.is_none());
}

#[test]
fn test_identity_override_trailing_newline_validates_and_derives_trimmed() {
    // A YAML block scalar (`identity: |`) yields a trailing newline. Validation
    // and derivation both trim first, so they must agree on the same key.
    let config = default_config_with_identity("github.com/owner/repo\n");
    assert!(config.validate().is_ok());
    let tmp = TempDir::new().unwrap();
    assert_eq!(
        key_of(&derive_identity(tmp.path(), &config)),
        Some("github.com/owner/repo")
    );
}

#[test]
fn test_derive_identity_override_with_control_chars_returns_none() {
    // Defensive: an override carrying control characters must not become
    // an identity key, matching `RepoConfig::validate`'s rejection.
    let tmp = TempDir::new().unwrap();
    let config = default_config_with_identity("github.com/owner/\u{001b}[31mrepo");
    let identity = derive_identity(tmp.path(), &config);
    assert!(identity.is_none());
}

#[test]
fn test_normalize_origin_url_key_longer_than_512_bytes_returns_none() {
    // A path whose `host/owner/repo` key exceeds 512 bytes must be
    // rejected (issue #29: identity keys are length-bounded).
    let long_segment = "a".repeat(700);
    let url = format!("git@github.com:owner/{long_segment}");
    assert_eq!(
        normalize_origin_url(&url),
        None,
        "keys longer than 512 bytes must normalize to None"
    );
    // 512 bytes or under is still accepted.
    let short_segment = "a".repeat(400);
    let url = format!("git@github.com:owner/{short_segment}");
    assert_eq!(
        normalize_origin_url(&url).as_deref(),
        Some(format!("github.com/owner/{short_segment}").as_str()),
        "a key within the 512-byte bound must still normalize"
    );
}

#[test]
fn test_normalize_origin_url_key_bound_is_exact_on_final_string() {
    // `github.com/owner/` is 17 bytes, so a 495-byte segment yields a
    // 512-byte key (accepted) and 496 bytes yields 513 (rejected).
    let at_bound = "a".repeat(495);
    let key = normalize_origin_url(&format!("git@github.com:owner/{at_bound}")).unwrap();
    assert_eq!(key.len(), MAX_KEY_LEN);
    let over_bound = "a".repeat(496);
    assert_eq!(
        normalize_origin_url(&format!("git@github.com:owner/{over_bound}")),
        None
    );
}

#[test]
fn test_derive_identity_override_longer_than_512_bytes_returns_none() {
    // A config override above 512 bytes is rejected (issue #29), matching
    // `RepoConfig::validate`'s rejection at config load.
    let tmp = TempDir::new().unwrap();
    let config = default_config_with_identity(&"a".repeat(700));
    let identity = derive_identity(tmp.path(), &config);
    assert!(identity.is_none());
}

#[test]
fn test_derive_identity_from_origin_remote() {
    let tmp = TempDir::new().unwrap();
    let _ = make_repo_with_remote(tmp.path(), "git@github.com:owner/repo");
    let config = RepoConfig::default();
    let identity = derive_identity(tmp.path(), &config);
    assert_eq!(key_of(&identity), Some("github.com/owner/repo"));
}

#[test]
fn test_derive_identity_repo_without_origin_returns_none() {
    let tmp = TempDir::new().unwrap();
    let _ = git2::Repository::init(tmp.path()).unwrap();
    let config = RepoConfig::default();
    let identity = derive_identity(tmp.path(), &config);
    assert!(identity.is_none());
}

#[test]
fn test_derive_identity_bare_repo_returns_none() {
    let tmp = TempDir::new().unwrap();
    let _ = git2::Repository::init_bare(tmp.path()).unwrap();
    let config = RepoConfig::default();
    let identity = derive_identity(tmp.path(), &config);
    assert!(identity.is_none());
}

#[test]
fn test_derive_identity_non_repo_returns_none() {
    let tmp = TempDir::new().unwrap();
    let config = RepoConfig::default();
    let identity = derive_identity(tmp.path(), &config);
    assert!(identity.is_none());
}
