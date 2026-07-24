//! EasyNet CLI — core identity value objects
//! =========================================
//!
//! File: src/core/identity/mod.rs
//! Description: Product-neutral runtime identity validation values.
//!
//! Protocol Responsibility:
//! - Distinguish syntactically valid URAs from admissible runtime identities.
//! - Reject the legacy all-zero principal placeholder before tuple signing.
//!
//! Implementation Approach:
//! - Normalize and validate caller/callee/subject URAs through one value object.
//! - Keep host identity, signer custody, and key resolution outside Core.
//!
//! Usage Contract:
//! - Runtime tuple constructors must use `RuntimeIdentityUra`.
//! - Raw principal-id fields use the exact/embedded sentinel predicates.
//!
//! Architectural Position:
//! - Core identity semantics below SDK/daemon ingress and above URA syntax.

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

/// A normalized URA that is syntactically valid and contains no placeholder
/// principal.
///
/// This value object is intentionally broader than a User URA: the Invocation
/// tuple also carries Agent, Device, Ability, Resource, and Authority
/// identities. Its invariant is exactly what every tuple identity field shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIdentityUra {
    ura: String,
    kind: crate::core::ura::URAKind,
    realm: String,
}

impl RuntimeIdentityUra {
    pub fn parse(value: impl AsRef<str>) -> Result<Self, RuntimeIdentityUraError> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(RuntimeIdentityUraError::Empty);
        }
        if contains_all_zero_principal_placeholder(value) {
            return Err(RuntimeIdentityUraError::AllZeroPrincipalPlaceholder);
        }
        let parsed = crate::core::ura::parse_ura(value)
            .map_err(|error| RuntimeIdentityUraError::InvalidSyntax(error.to_string()))?;
        Ok(Self {
            ura: value.to_string(),
            kind: parsed.kind,
            realm: parsed.realm,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.ura
    }

    #[must_use]
    pub fn kind(&self) -> crate::core::ura::URAKind {
        self.kind
    }

    #[must_use]
    pub fn realm(&self) -> &str {
        &self.realm
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.ura
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeIdentityUraError {
    Empty,
    AllZeroPrincipalPlaceholder,
    InvalidSyntax(String),
}

impl std::fmt::Display for RuntimeIdentityUraError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("must not be empty"),
            Self::AllZeroPrincipalPlaceholder => {
                formatter.write_str("must not contain the all-zero principal placeholder")
            }
            Self::InvalidSyntax(error) => write!(formatter, "is not a valid URA: {error}"),
        }
    }
}

impl std::error::Error for RuntimeIdentityUraError {}

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

    #[test]
    fn runtime_identity_ura_normalizes_and_rejects_placeholder_principals() {
        let identity = RuntimeIdentityUra::parse("  easynet:///r/acme/user/alice  ")
            .expect("canonical non-zero User URA");
        assert_eq!(identity.as_str(), "easynet:///r/acme/user/alice");
        assert_eq!(identity.kind(), crate::core::ura::URAKind::User);
        assert_eq!(identity.realm(), "acme");
        assert_eq!(
            identity.clone().into_string(),
            "easynet:///r/acme/user/alice"
        );

        assert_eq!(
            RuntimeIdentityUra::parse(
                "easynet:///r/acme/resource/user.00000000-0000-0000-0000-000000000000/runtime-state/read"
            ),
            Err(RuntimeIdentityUraError::AllZeroPrincipalPlaceholder)
        );
        assert_eq!(
            RuntimeIdentityUra::parse(" "),
            Err(RuntimeIdentityUraError::Empty)
        );
        assert!(matches!(
            RuntimeIdentityUra::parse("not-a-ura"),
            Err(RuntimeIdentityUraError::InvalidSyntax(_))
        ));
    }
}
