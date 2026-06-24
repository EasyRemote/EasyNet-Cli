// EasyNet CLI — Runtime failure code classifier
// =============================================
//
// File: src/runtime/failure_codes.rs
// Description: Shared classifier for product/runtime failure reason strings.
//
// Protocol Responsibility:
// - Owns EasyNet product-side extraction of stable failure reason codes from
//   daemon/backend/Axon messages. It does not define Axon protocol enums.
//
// Implementation Approach:
// - Keep this as a small value object with pure functions. Callers provide the
//   stable fallback code for their own state machine; this classifier only
//   upgrades to a more precise proven runtime/admission code.
//
// Usage Contract:
// - Never invent a specific code unless it is either present as a token in the
//   reason or matched by a phrase emitted by current daemon/backend surfaces.
//
// Architectural Position:
// - Runtime product semantics shared by CLI state snapshots and daemon receipt
//   producers.

const SPECIFIC_PREFIXES: &[&str] = &[
    "AXON_",
    "CALLER_",
    "AUTHORITY_",
    "NONCE_",
    "ENVELOPE_",
    "TARGET_",
    "AGENT_",
    "DEVICE_",
    "PRESENCE_",
    "RESOLVE_",
    "ROUTE_",
    "INVOCATION_",
    "POLICY_",
    "DENDRITE_",
    "BRIDGE_",
];

const NON_FAILURE_TOKENS: &[&str] = &["INVOCATION_ID", "REQUEST_ID", "CALL_ID"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureClass {
    pub stage: RuntimeErrorStage,
    pub security_class: RuntimeSecurityClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeErrorStage {
    CallerAuthentication,
    AuthorityValidation,
    GlobalAdmission,
    AbilityResolution,
    Transport,
    Quota,
    AbilityPolicy,
    RequestValidation,
    Execution,
}

impl RuntimeErrorStage {
    pub fn as_str_name(self) -> &'static str {
        match self {
            Self::CallerAuthentication => "ERROR_STAGE_CALLER_AUTHENTICATION",
            Self::AuthorityValidation => "ERROR_STAGE_AUTHORITY_VALIDATION",
            Self::GlobalAdmission => "ERROR_STAGE_GLOBAL_ADMISSION",
            Self::AbilityResolution => "ERROR_STAGE_ABILITY_RESOLUTION",
            Self::Transport => "ERROR_STAGE_TRANSPORT",
            Self::Quota => "ERROR_STAGE_QUOTA",
            Self::AbilityPolicy => "ERROR_STAGE_ABILITY_POLICY",
            Self::RequestValidation => "ERROR_STAGE_REQUEST_VALIDATION",
            Self::Execution => "ERROR_STAGE_EXECUTION",
        }
    }

    pub fn axon_number(self) -> i32 {
        match self {
            Self::Transport => 1,
            Self::GlobalAdmission => 2,
            Self::CallerAuthentication => 3,
            Self::AuthorityValidation => 4,
            Self::Quota => 6,
            Self::AbilityResolution => 7,
            Self::AbilityPolicy => 8,
            Self::RequestValidation => 9,
            Self::Execution => 10,
        }
    }

    #[cfg(feature = "axon-pb")]
    pub fn to_axon_pb(self) -> easynet_axon::pb::axon::v1::ErrorStage {
        use easynet_axon::pb::axon::v1::ErrorStage;

        match self {
            Self::CallerAuthentication => ErrorStage::CallerAuthentication,
            Self::AuthorityValidation => ErrorStage::AuthorityValidation,
            Self::GlobalAdmission => ErrorStage::GlobalAdmission,
            Self::AbilityResolution => ErrorStage::AbilityResolution,
            Self::Transport => ErrorStage::Transport,
            Self::Quota => ErrorStage::Quota,
            Self::AbilityPolicy => ErrorStage::AbilityPolicy,
            Self::RequestValidation => ErrorStage::RequestValidation,
            Self::Execution => ErrorStage::Execution,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSecurityClass {
    Unspecified,
    Authentication,
    Authority,
    Transport,
    Resource,
    Authorization,
    Internal,
}

impl RuntimeSecurityClass {
    pub fn as_str_name(self) -> &'static str {
        match self {
            Self::Unspecified => "SECURITY_CLASS_UNSPECIFIED",
            Self::Authentication => "SECURITY_CLASS_AUTHENTICATION",
            Self::Authority => "SECURITY_CLASS_AUTHORITY",
            Self::Transport => "SECURITY_CLASS_TRANSPORT",
            Self::Resource => "SECURITY_CLASS_RESOURCE",
            Self::Authorization => "SECURITY_CLASS_AUTHORIZATION",
            Self::Internal => "SECURITY_CLASS_INTERNAL",
        }
    }

    pub fn axon_number(self) -> i32 {
        match self {
            Self::Unspecified => 0,
            Self::Authentication => 2,
            Self::Authority => 3,
            Self::Authorization => 5,
            Self::Resource => 6,
            Self::Transport => 7,
            Self::Internal => 8,
        }
    }

    #[cfg(feature = "axon-pb")]
    pub fn to_axon_pb(self) -> easynet_axon::pb::axon::v1::SecurityClass {
        use easynet_axon::pb::axon::v1::SecurityClass;

        match self {
            Self::Unspecified => SecurityClass::Unspecified,
            Self::Authentication => SecurityClass::Authentication,
            Self::Authority => SecurityClass::Authority,
            Self::Transport => SecurityClass::Transport,
            Self::Resource => SecurityClass::Resource,
            Self::Authorization => SecurityClass::Authorization,
            Self::Internal => SecurityClass::Internal,
        }
    }
}

pub struct FailureCodeClassifier;

impl FailureCodeClassifier {
    pub fn classify_or(reason: &str, fallback: &str) -> String {
        Self::extract(reason).unwrap_or_else(|| Self::normalize(fallback, fallback))
    }

    pub fn explicit_or_reason(explicit: Option<&str>, reason: &str, fallback: &str) -> String {
        explicit
            .map(|code| Self::normalize(code, fallback))
            .filter(|code| !code.is_empty())
            .unwrap_or_else(|| Self::classify_or(reason, fallback))
    }

    pub fn extract(reason: &str) -> Option<String> {
        let lowered = reason.to_ascii_lowercase();
        if lowered.contains("not in presenceregistry") {
            return Some("TARGET_NOT_IN_PRESENCE_REGISTRY".to_string());
        }
        if lowered.contains("not advertised on this hub") {
            return Some("AGENT_NOT_ADVERTISED".to_string());
        }
        if lowered.contains("device") && lowered.contains("removed") {
            return Some("DEVICE_REMOVED".to_string());
        }
        if lowered.contains("dendrite bridge library not found")
            || lowered.contains("bridge library not found")
        {
            return Some("DENDRITE_BRIDGE_LIBRARY_NOT_FOUND".to_string());
        }

        reason
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .map(|token| Self::normalize(token, ""))
            .find(|token| Self::is_specific(token))
    }

    pub fn normalize(candidate: &str, fallback: &str) -> String {
        let raw = if candidate.trim().is_empty() {
            fallback
        } else {
            candidate.trim()
        };
        let mut out = String::with_capacity(raw.len());
        let mut prev_underscore = false;
        for ch in raw.chars() {
            let mapped = if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            };
            if mapped == '_' {
                if !prev_underscore && !out.is_empty() {
                    out.push(mapped);
                    prev_underscore = true;
                }
                continue;
            }
            out.push(mapped);
            prev_underscore = false;
        }
        let trimmed = out.trim_matches('_');
        if trimmed.is_empty() {
            fallback.to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub fn classify_error_class(code: &str) -> FailureClass {
        use RuntimeErrorStage as Stage;
        use RuntimeSecurityClass as Security;

        let code = code.trim().to_ascii_uppercase();
        let (stage, security_class) = if code.starts_with("CALLER_") {
            (Stage::CallerAuthentication, Security::Authentication)
        } else if code.starts_with("AUTHORITY_") {
            (Stage::AuthorityValidation, Security::Authority)
        } else if code.starts_with("NONCE_") || code.starts_with("ENVELOPE_") {
            (Stage::GlobalAdmission, Security::Authentication)
        } else if code.starts_with("ROUTE_") || code.starts_with("RESOLVE_") {
            (Stage::AbilityResolution, Security::Unspecified)
        } else if code.starts_with("TARGET_")
            || code.starts_with("PRESENCE_")
            || code.starts_with("DEVICE_")
        {
            (Stage::Transport, Security::Transport)
        } else if code.starts_with("AGENT_") {
            (Stage::AbilityResolution, Security::Resource)
        } else if code.contains("QUOTA") {
            (Stage::Quota, Security::Authorization)
        } else if code.contains("POLICY") || code.contains("AUTHORIZATION") {
            (Stage::AbilityPolicy, Security::Authorization)
        } else if code.contains("VALIDATION") || code.contains("INVALID_ARGUMENT") {
            (Stage::RequestValidation, Security::Unspecified)
        } else if code.starts_with("DENDRITE_") || code.starts_with("BRIDGE_") {
            (Stage::Transport, Security::Internal)
        } else {
            (Stage::Execution, Security::Unspecified)
        };

        FailureClass {
            stage,
            security_class,
        }
    }

    fn is_specific(token: &str) -> bool {
        token.contains('_')
            && !NON_FAILURE_TOKENS.contains(&token)
            && SPECIFIC_PREFIXES
                .iter()
                .any(|prefix| token.starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::FailureCodeClassifier;

    #[test]
    fn extracts_admission_reason_token() {
        assert_eq!(
            FailureCodeClassifier::extract("CALLER_SIGNATURE_INVALID: rejected <self>.session")
                .as_deref(),
            Some("CALLER_SIGNATURE_INVALID")
        );
    }

    #[test]
    fn extracts_presence_registry_phrase() {
        assert_eq!(
            FailureCodeClassifier::extract("target device is not in PresenceRegistry").as_deref(),
            Some("TARGET_NOT_IN_PRESENCE_REGISTRY")
        );
    }

    #[test]
    fn explicit_code_wins_and_normalizes() {
        assert_eq!(
            FailureCodeClassifier::explicit_or_reason(
                Some("disk full"),
                "CALLER_SIGNATURE_INVALID",
                "INVOCATION_FAILED",
            ),
            "DISK_FULL"
        );
    }

    #[test]
    fn falls_back_when_reason_has_no_specific_code() {
        assert_eq!(
            FailureCodeClassifier::classify_or("pty exited with status 1", "INVOCATION_FAILED"),
            "INVOCATION_FAILED"
        );
    }

    #[test]
    fn ignores_invocation_field_names() {
        assert_eq!(
            FailureCodeClassifier::classify_or(
                "Axon invocation_id=abc ended without terminal event",
                "INVOCATION_FAILED",
            ),
            "INVOCATION_FAILED"
        );
    }

    #[test]
    fn extracts_route_negative_reason_token() {
        assert_eq!(
            FailureCodeClassifier::extract("ROUTE_NEGATIVE: namespace.resolve NXDOMAIN").as_deref(),
            Some("ROUTE_NEGATIVE")
        );
    }

    #[test]
    fn extracts_dendrite_bridge_library_missing_phrase() {
        assert_eq!(
            FailureCodeClassifier::extract(
                "bridge: dendrite bridge library not found; set EASYNET_DENDRITE_BRIDGE_LIB",
            )
            .as_deref(),
            Some("DENDRITE_BRIDGE_LIBRARY_NOT_FOUND")
        );
    }

    #[test]
    fn ignores_non_failure_identity_tokens() {
        assert_eq!(
            FailureCodeClassifier::extract("missing invocation id in local runtime result"),
            None
        );
        assert_eq!(
            FailureCodeClassifier::extract("request_id not assigned before terminal"),
            None
        );
    }
}
