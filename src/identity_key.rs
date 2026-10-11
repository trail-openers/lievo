// Identity-key bounds shared by config validation and identity derivation.
//
// A leaf module: it imports nothing from the crate, so `config` and `identity`
// can both depend on it without a module cycle.

/// Upper bound (in bytes) for an identity key. Keys are `host/owner/repo`
/// URL shapes; anything longer is not a real remote (or an adversarial
/// override) and is rejected rather than persisted or printed.
pub(crate) const MAX_KEY_LEN: usize = 512;

/// Whether `s` is acceptable as an identity key from any source (derived
/// remote or config override): no control characters and within
/// `MAX_KEY_LEN` bytes. Shape checks (owner/repo, path segments) are separate.
pub(crate) fn is_valid_identity_key(s: &str) -> bool {
    !s.chars().any(|c| c.is_control()) && s.len() <= MAX_KEY_LEN
}
