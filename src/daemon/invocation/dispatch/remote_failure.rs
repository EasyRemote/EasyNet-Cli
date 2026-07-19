//! Typed projection from a remote runtime failure into gRPC transport status.
//!
//! Presence carriers already return a stable SessionFailure. The forwarding
//! daemon must preserve that semantic class instead of flattening every
//! pre-finalization rejection into FAILED_PRECONDITION or UNAVAILABLE.

use tonic::{Code, Status};

use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;

pub(crate) fn status_from_remote_failure(
    context: &str,
    raw_error: &str,
    failure: Option<&SessionFailure>,
) -> Status {
    let detail = failure
        .map(SessionFailure::status_detail)
        .filter(|detail| !detail.trim().is_empty())
        .unwrap_or_else(|| raw_error.trim().to_string());
    let code = failure
        .map(|failure| status_code_for_failure(&failure.code, &detail))
        .unwrap_or_else(|| status_code_for_failure("", &detail));
    Status::new(code, format!("{context}: {detail}"))
}

pub(crate) fn is_admission_denial_message(message: &str) -> bool {
    message.contains("POLICY_DENIED")
        || message.contains("AUTHORITY_DENIED")
        || message.contains("SIGNATURE_DENIED")
        || message.contains("AXON_CALLER_SIGNATURE_INVALID")
        || message.contains("SIGNED_DESCRIPTOR_REF_")
        || message.contains("SIGNED_ENVELOPE_ROUTE_MUTATION")
}

fn status_code_for_failure(raw_code: &str, detail: &str) -> Code {
    let code = raw_code.trim().to_ascii_uppercase();
    if is_admission_denial_message(detail)
        || code == "PERMISSION_DENIED"
        || code.starts_with("AUTHORITY_")
        || code.starts_with("POLICY_")
        || code.starts_with("SIGNATURE_")
        || (code.starts_with("CALLER_")
            && !matches!(
                code.as_str(),
                "CALLER_URA_MISSING" | "CALLER_URA_INVALID" | "CALLER_NONCE_MISSING"
            ))
        || matches!(
            code.as_str(),
            "ABILITY_FORBIDDEN" | "ABILITY_ROLE_RESTRICTED" | "ABILITY_REALM_RESTRICTED"
        )
    {
        return Code::PermissionDenied;
    }
    if matches!(code.as_str(), "NOT_FOUND" | "ABILITY_NOT_FOUND") {
        return Code::NotFound;
    }
    if code.starts_with("REQUEST_")
        || matches!(
            code.as_str(),
            "CALLER_URA_MISSING" | "CALLER_URA_INVALID" | "CALLER_NONCE_MISSING"
        )
    {
        return Code::InvalidArgument;
    }
    if matches!(
        code.as_str(),
        "QUOTA_EXCEEDED" | "RATE_LIMITED" | "RESOURCE_EXHAUSTED"
    ) {
        return Code::ResourceExhausted;
    }
    if code == "UPSTREAM_TIMEOUT" {
        return Code::DeadlineExceeded;
    }
    if matches!(code.as_str(), "UPSTREAM_FAILURE" | "TARGET_OFFLINE") {
        return Code::Unavailable;
    }
    if matches!(
        code.as_str(),
        "INTERNAL_ERROR" | "INTERNAL_INVARIANT_VIOLATION"
    ) {
        return Code::Internal;
    }
    Code::FailedPrecondition
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(code: &str, message: &str) -> SessionFailure {
        SessionFailure::new(code, message, false, 0, 0)
    }

    #[test]
    fn preserves_remote_authority_denial_as_permission_denied() {
        let failure = failure(
            "AUTHORITY_DENIED",
            "AUTHORITY_SUBJECT_MISMATCH: session does not admit subject",
        );
        let status = status_from_remote_failure("remote Invoke", "ignored", Some(&failure));
        assert_eq!(status.code(), Code::PermissionDenied);
        assert!(status.message().contains("AUTHORITY_SUBJECT_MISMATCH"));
    }

    #[test]
    fn preserves_remote_resolution_and_payload_classes() {
        let missing = failure("ABILITY_NOT_FOUND", "descriptor missing");
        assert_eq!(
            status_from_remote_failure("remote Invoke", "ignored", Some(&missing)).code(),
            Code::NotFound
        );
        let invalid = failure("REQUEST_PAYLOAD_INVALID", "bad payload");
        assert_eq!(
            status_from_remote_failure("remote Invoke", "ignored", Some(&invalid)).code(),
            Code::InvalidArgument
        );
    }

    #[test]
    fn classifies_legacy_typed_detail_without_downgrading_security() {
        let status = status_from_remote_failure(
            "remote Invoke",
            "AUTHORITY_DENIED: target rejected caller",
            None,
        );
        assert_eq!(status.code(), Code::PermissionDenied);
    }
}
