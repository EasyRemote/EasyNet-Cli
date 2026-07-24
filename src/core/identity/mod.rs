//! Core identity value objects.
//!
//! Host identity, signing handles, key resolution, and vault access are daemon
//! policy and live under `daemon::identity`, `daemon::trust`, and
//! `daemon::keyring`. This module is reserved for pure identity value objects.

/// Placeholder principal id used by legacy/dummy fixtures and never admitted
/// as a runtime identity fact.
///
/// This sentinel is a core identity validity rule. Callers may keep local error
/// types and remediation messages, but they must not duplicate the literal or
/// implement a parallel string check.
pub const ALL_ZERO_PRINCIPAL_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Return true when `value` is exactly the all-zero principal placeholder after
/// trimming caller-supplied whitespace.
#[must_use]
pub fn is_all_zero_principal_id(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case(ALL_ZERO_PRINCIPAL_ID)
}

/// Return true when `value` embeds the all-zero principal placeholder inside a
/// larger identity/URA field.
#[must_use]
pub fn contains_all_zero_principal_placeholder(value: &str) -> bool {
    value
        .trim()
        .to_ascii_lowercase()
        .contains(ALL_ZERO_PRINCIPAL_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_all_zero_principal_id_trims_and_ignores_case() {
        assert!(is_all_zero_principal_id(
            "  00000000-0000-0000-0000-000000000000  "
        ));
        assert!(is_all_zero_principal_id(
            "00000000-0000-0000-0000-000000000000"
        ));
        assert!(!is_all_zero_principal_id(
            "user.00000000-0000-0000-0000-000000000000"
        ));
    }

    #[test]
    fn embedded_all_zero_principal_placeholder_detects_ura_fields() {
        assert!(contains_all_zero_principal_placeholder(
            "easynet:///r/acme/resource/user.00000000-0000-0000-0000-000000000000/session/s1"
        ));
        assert!(!contains_all_zero_principal_placeholder(
            "easynet:///r/acme/user/alice"
        ));
    }
}
