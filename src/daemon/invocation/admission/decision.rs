// EasyNet CLI — RFC-014 admission decision model
// =================================================
//
// File: src/daemon/invocation/admission/decision.rs
// Description: Stable RFC-014 decision DTOs shared by daemon admission,
//              policy storage, governance abilities, and SDK projections.
//
// Protocol Responsibility:
// Keep signature verification, policy evaluation, authority proof decisions,
// and trace projection as typed runtime facts without changing the public
// Axon Invocation tuple.
//
// Implementation Approach:
// The module is pure data plus small constructors. It deliberately contains
// no gRPC, filesystem, keyring, or handler logic so every caller serializes
// the same canonical RFC-014 shape.
//
// Usage Contract:
// Public-facing wrappers may keep legacy outer error codes, but diagnostic
// payloads must carry these concrete reasons.
//
// Architectural Position:
// Daemon admission domain model. SDKs mirror these names and enum values;
// product repositories consume them rather than redefining policy semantics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    User,
    Token,
    Hub,
    Device,
    Service,
    Automation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenClass {
    HubLink,
    BrowserSession,
    DevicePairing,
    Automation,
    ThirdParty,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessAction {
    Read,
    Invoke,
    Stream,
    Manage,
    Grant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerSource {
    Subject,
    Callee,
    Device,
    Session,
    Unresolved,
}

impl OwnerSource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Subject => "subject",
            Self::Callee => "callee",
            Self::Device => "device",
            Self::Session => "session",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OwnerResolution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_ura: Option<String>,
    pub owner_source: OwnerSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audit_warnings: Vec<String>,
}

impl OwnerResolution {
    #[must_use]
    pub fn unresolved(reason: impl Into<String>) -> Self {
        Self {
            owner_user_id: None,
            owner_ura: None,
            owner_source: OwnerSource::Unresolved,
            audit_warnings: vec![reason.into()],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecisionOutcome {
    Allow,
    Deny,
    Prompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PolicyDecisionReason {
    OwnerAllow,
    ExplicitGrantAllow,
    ExplicitDeny,
    HubTokenReadAllow,
    MissingGrant,
    GrantReconfirmationRequired,
    TokenScopeDenied,
    MissingOriginCaller,
    CallerNotOwner,
    OwnerUnresolved,
    InteractiveApprovalRequired,
    NonInteractiveDeny,
    AuthorityProofExpired,
    AuthorityProofMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PolicyDecision {
    pub decision: PolicyDecisionOutcome,
    pub reason: PolicyDecisionReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    pub owner_source: OwnerSource,
    pub caller_ura: String,
    pub principal_kind: PrincipalKind,
    pub principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    pub callee_ura: String,
    pub subject_ura: String,
    pub ability_ura: String,
    pub action: AccessAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejector_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_rule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority_proof_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureDecisionOutcome {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SignatureDecisionReason {
    SignatureValid,
    CallerSignatureMissing,
    CallerKeyNotFound,
    CallerKeyRevoked,
    CallerSignatureVerifyFailed,
    CanonicalHashMismatch,
    SignedDescriptorRefMissing,
    SignedDescriptorRefMismatch,
    SignedEnvelopeRouteMutation,
    FederatedKeyResolveFailed,
}

impl SignatureDecisionReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SignatureValid => "SIGNATURE_VALID",
            Self::CallerSignatureMissing => "CALLER_SIGNATURE_MISSING",
            Self::CallerKeyNotFound => "CALLER_KEY_NOT_FOUND",
            Self::CallerKeyRevoked => "CALLER_KEY_REVOKED",
            Self::CallerSignatureVerifyFailed => "CALLER_SIGNATURE_VERIFY_FAILED",
            Self::CanonicalHashMismatch => "CANONICAL_HASH_MISMATCH",
            Self::SignedDescriptorRefMissing => "SIGNED_DESCRIPTOR_REF_MISSING",
            Self::SignedDescriptorRefMismatch => "SIGNED_DESCRIPTOR_REF_MISMATCH",
            Self::SignedEnvelopeRouteMutation => "SIGNED_ENVELOPE_ROUTE_MUTATION",
            Self::FederatedKeyResolveFailed => "FEDERATED_KEY_RESOLVE_FAILED",
        }
    }

    #[must_use]
    pub fn from_admission_detail(detail: &str) -> Self {
        let upper = detail.to_ascii_uppercase();
        if upper.contains(Self::CallerKeyNotFound.as_str()) {
            return Self::CallerKeyNotFound;
        }
        if upper.contains(Self::CallerKeyRevoked.as_str()) {
            return Self::CallerKeyRevoked;
        }
        if upper.contains(Self::CanonicalHashMismatch.as_str())
            || upper.contains("CANONICALIZATION")
        {
            return Self::CanonicalHashMismatch;
        }
        if upper.contains(Self::SignedDescriptorRefMissing.as_str()) {
            return Self::SignedDescriptorRefMissing;
        }
        if upper.contains(Self::SignedDescriptorRefMismatch.as_str()) {
            return Self::SignedDescriptorRefMismatch;
        }
        if upper.contains(Self::SignedEnvelopeRouteMutation.as_str()) {
            return Self::SignedEnvelopeRouteMutation;
        }
        if upper.contains(Self::FederatedKeyResolveFailed.as_str()) {
            return Self::FederatedKeyResolveFailed;
        }
        if upper.contains("CALLER_SIGNATURE_MISSING") || upper.contains("SIGNATURE_MISSING") {
            return Self::CallerSignatureMissing;
        }
        Self::CallerSignatureVerifyFailed
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SignatureDecision {
    pub decision: SignatureDecisionOutcome,
    pub reason: SignatureDecisionReason,
    pub caller_ura: String,
    pub callee_ura: String,
    pub ability_ura: String,
    pub subject_ura: String,
    pub canonical_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_key_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presented_pubkey_fingerprint: Option<String>,
    pub verifier_ura: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRequestStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    Cancelled,
}

impl PermissionRequestStatus {
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Approved | Self::Denied | Self::Expired | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLifetime {
    Once,
    Session,
    Ttl,
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub owner_user_id: String,
    pub caller_ura: String,
    pub principal_kind: PrincipalKind,
    pub principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_class: Option<TokenClass>,
    pub callee_ura: String,
    pub subject_ura: String,
    pub ability_ura: String,
    pub action: AccessAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_hash: Option<String>,
    pub requested_lifetimes: Vec<PermissionLifetime>,
    pub status: PermissionRequestStatus,
    pub created_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolver_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_lifetime: Option<PermissionLifetime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_grant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority_proof_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStage {
    Prepared,
    RouteSelected,
    TargetAdmission,
    SignatureVerified,
    PolicyChecked,
    AuthorityVerified,
    Admitted,
    Dispatched,
    Executed,
    Receipted,
    SignatureDenied,
    PolicyDenied,
    AuthorityDenied,
    RouteUnavailable,
    ExecutionFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildFailureClass {
    DownstreamDependencyDenied,
    DownstreamDependencyUnavailable,
    DownstreamDependencyFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RedactionReason {
    ObserverNotAuthorizedForChildEdge,
    ChildTopologyPrivate,
    SubjectPrivate,
    KeyMaterialPrivate,
    GrantPrivate,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AbilityCallTrace {
    pub invocation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_invocation_id: Option<String>,
    pub root_invocation_id: String,
    pub caller_ura: String,
    pub callee_ura: String,
    pub subject_ura: String,
    pub ability_ura: String,
    pub action: AccessAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_host_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejector_ura: Option<String>,
    pub stage: TraceStage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_decision: Option<SignatureDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision: Option<PolicyDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority_proof_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_failure_class: Option<ChildFailureClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction_reason: Option<RedactionReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AbilityCallTrace>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AdmissionExplainResult {
    pub observer_ura: String,
    pub redacted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_trace: Option<AbilityCallTrace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_decision: Option<SignatureDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision: Option<PolicyDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejector_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction_reason: Option<RedactionReason>,
}

#[cfg(test)]
mod tests {
    use super::SignatureDecisionReason;

    #[test]
    fn signature_reason_parser_preserves_specific_legacy_detail() {
        assert_eq!(
            SignatureDecisionReason::from_admission_detail(
                "AXON_CALLER_SIGNATURE_INVALID: CALLER_KEY_NOT_FOUND"
            ),
            SignatureDecisionReason::CallerKeyNotFound
        );
        assert_eq!(
            SignatureDecisionReason::from_admission_detail(
                "SIGNED_DESCRIPTOR_REF_MISMATCH: route changed"
            ),
            SignatureDecisionReason::SignedDescriptorRefMismatch
        );
        assert_eq!(
            SignatureDecisionReason::from_admission_detail("opaque bad signature"),
            SignatureDecisionReason::CallerSignatureVerifyFailed
        );
    }
}
