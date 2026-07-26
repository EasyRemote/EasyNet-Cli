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

/// Canonical subject for user-owned runtime-state read projections.
///
/// This is a runtime identity value object, not a product receipt type. History,
/// catalogue, and status reads may all use this subject shape when they need a
/// user-scoped read subject instead of a target-owned caller/callee subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStateReadSubject {
    subject_ura: String,
    realm: String,
    user_id: String,
}

impl RuntimeStateReadSubject {
    pub const RESOURCE_PATH: &'static str = "runtime-state/read";

    pub fn new(
        realm: impl AsRef<str>,
        user_id: impl AsRef<str>,
    ) -> Result<Self, RuntimeStateReadSubjectError> {
        let realm = realm.as_ref().trim();
        if realm.is_empty() {
            return Err(RuntimeStateReadSubjectError::EmptyRealm);
        }
        let user_id = user_id.as_ref().trim();
        if user_id.is_empty() {
            return Err(RuntimeStateReadSubjectError::EmptyUserId);
        }
        if contains_all_zero_principal_placeholder(user_id) {
            return Err(RuntimeStateReadSubjectError::AllZeroPrincipalPlaceholder);
        }
        let subject_ura = crate::core::ura::resource_dot_ura(
            realm,
            &format!("user.{user_id}"),
            Self::RESOURCE_PATH,
        );
        Self::parse(subject_ura)
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, RuntimeStateReadSubjectError> {
        let subject_ura = value.as_ref().trim();
        if subject_ura.is_empty() {
            return Err(RuntimeStateReadSubjectError::Empty);
        }
        if contains_all_zero_principal_placeholder(subject_ura) {
            return Err(RuntimeStateReadSubjectError::AllZeroPrincipalPlaceholder);
        }
        let parsed = crate::core::ura::parse_ura(subject_ura)
            .map_err(|error| RuntimeStateReadSubjectError::InvalidSyntax(error.to_string()))?;
        if parsed.kind != crate::core::ura::URAKind::Resource {
            return Err(RuntimeStateReadSubjectError::NotResource);
        }
        let Some(owner) = parsed.resource_owner_id() else {
            return Err(RuntimeStateReadSubjectError::NotUserOwnedRuntimeStateRead);
        };
        let Some(user_id) = owner.strip_prefix("user.") else {
            return Err(RuntimeStateReadSubjectError::NotUserOwnedRuntimeStateRead);
        };
        if user_id.trim().is_empty()
            || user_id.contains('.')
            || contains_all_zero_principal_placeholder(user_id)
        {
            return Err(RuntimeStateReadSubjectError::NotUserOwnedRuntimeStateRead);
        }
        if parsed.resource_path() != Some(Self::RESOURCE_PATH) {
            return Err(RuntimeStateReadSubjectError::NotUserOwnedRuntimeStateRead);
        }
        let user_id = user_id.to_string();
        Ok(Self {
            subject_ura: subject_ura.to_string(),
            realm: parsed.realm,
            user_id,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.subject_ura
    }

    #[must_use]
    pub fn realm(&self) -> &str {
        &self.realm
    }

    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.subject_ura
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStateReadSubjectError {
    Empty,
    EmptyRealm,
    EmptyUserId,
    AllZeroPrincipalPlaceholder,
    InvalidSyntax(String),
    NotResource,
    NotUserOwnedRuntimeStateRead,
}

impl std::fmt::Display for RuntimeStateReadSubjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("must not be empty"),
            Self::EmptyRealm => formatter.write_str("realm must not be empty"),
            Self::EmptyUserId => formatter.write_str("user_id must not be empty"),
            Self::AllZeroPrincipalPlaceholder => {
                formatter.write_str("must not contain the all-zero principal placeholder")
            }
            Self::InvalidSyntax(error) => write!(formatter, "is not a valid URA: {error}"),
            Self::NotResource => formatter.write_str("must be a Resource URA"),
            Self::NotUserOwnedRuntimeStateRead => {
                formatter.write_str("must be a user-owned runtime-state read subject")
            }
        }
    }
}

impl std::error::Error for RuntimeStateReadSubjectError {}

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

    #[test]
    fn runtime_state_read_subject_projects_user_owned_read_facts() {
        let subject = RuntimeStateReadSubject::new(" acme ", " alice ")
            .expect("canonical runtime-state read subject");

        assert_eq!(
            subject.as_str(),
            "easynet:///r/acme/resource/user.alice/runtime-state/read"
        );
        assert_eq!(subject.realm(), "acme");
        assert_eq!(subject.user_id(), "alice");
        assert_eq!(
            subject.clone().into_string(),
            "easynet:///r/acme/resource/user.alice/runtime-state/read"
        );

        let parsed = RuntimeStateReadSubject::parse(
            "  easynet:///r/acme/resource/user.alice/runtime-state/read  ",
        )
        .expect("canonical runtime-state read subject parse");
        assert_eq!(parsed, subject);
    }

    #[test]
    fn runtime_state_read_subject_rejects_defaulted_or_retired_subjects() {
        assert_eq!(
            RuntimeStateReadSubject::new(" ", "alice"),
            Err(RuntimeStateReadSubjectError::EmptyRealm)
        );
        assert_eq!(
            RuntimeStateReadSubject::new("acme", " "),
            Err(RuntimeStateReadSubjectError::EmptyUserId)
        );
        assert_eq!(
            RuntimeStateReadSubject::parse(" "),
            Err(RuntimeStateReadSubjectError::Empty)
        );
        assert_eq!(
            RuntimeStateReadSubject::parse(
                "easynet:///r/acme/resource/user.00000000-0000-0000-0000-000000000000/runtime-state/read"
            ),
            Err(RuntimeStateReadSubjectError::AllZeroPrincipalPlaceholder)
        );
        assert_eq!(
            RuntimeStateReadSubject::parse("easynet:///r/acme/device/dev-a"),
            Err(RuntimeStateReadSubjectError::NotResource)
        );
        assert_eq!(
            RuntimeStateReadSubject::parse(
                "easynet:///r/acme/resource/user.alice/session/invocation_history"
            ),
            Err(RuntimeStateReadSubjectError::NotUserOwnedRuntimeStateRead)
        );
        assert_eq!(
            RuntimeStateReadSubject::parse(
                "easynet:///r/acme/resource/agent.alice.reader/runtime-state/read"
            ),
            Err(RuntimeStateReadSubjectError::NotUserOwnedRuntimeStateRead)
        );
        assert!(matches!(
            RuntimeStateReadSubject::parse("not-a-ura"),
            Err(RuntimeStateReadSubjectError::InvalidSyntax(_))
        ));
    }
}
