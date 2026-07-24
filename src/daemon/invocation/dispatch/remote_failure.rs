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
    let raw_detail = failure
        .map(SessionFailure::status_detail)
        .filter(|detail| !detail.trim().is_empty())
        .unwrap_or_else(|| raw_error.trim().to_string());
    let detail = canonical_remote_failure_detail(&raw_detail);
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
    if is_route_unavailable_message(&code, detail) {
        return Code::Unavailable;
    }
    if is_caller_signer_unavailable_message(&code, detail) {
        return Code::PermissionDenied;
    }
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

fn is_route_unavailable_message(code: &str, detail: &str) -> bool {
    let detail = detail.to_ascii_uppercase();
    code == "ROUTE_UNAVAILABLE"
        || code == "RUNTIME_ROUTE_UNAVAILABLE"
        || code == "ROUTE_NEGATIVE"
        || detail.contains("ROUTE_NEGATIVE")
        || detail.contains("NEGATIVE_REASON_NXDOMAIN")
        || detail.contains("OWNER IS NOT ONLINE")
}

fn is_caller_signer_unavailable_message(code: &str, detail: &str) -> bool {
    let detail = detail.to_ascii_uppercase();
    code == "CALLER_SIGNER_UNAVAILABLE"
        || detail.contains("CALLER_SIGNER_UNAVAILABLE")
        || detail.contains("CALLER SIGNER UNAVAILABLE")
        || detail.contains("REQUIRES A CALLER SIGNER")
        || detail.contains("KEYRING ENTRY NOT FOUND")
        || detail.contains("SELF-IDENTITY:")
}

fn canonical_remote_failure_detail(detail: &str) -> String {
    let detail = detail.trim();
    if is_caller_signer_unavailable_message("", detail) {
        return canonical_caller_signer_unavailable_detail(detail);
    }
    detail.to_string()
}

fn canonical_caller_signer_unavailable_detail(detail: &str) -> String {
    match caller_ura_from_signer_detail(detail) {
        Some(caller_ura) => format!(
            "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer for \
             `{caller_ura}`; load or provision that identity in the local key service"
        ),
        None => "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer; \
             load or provision that identity in the local key service"
            .to_string(),
    }
}

fn caller_ura_from_signer_detail(detail: &str) -> Option<&str> {
    let (_, tail) = detail.split_once("for `")?;
    let (caller_ura, _) = tail.split_once('`')?;
    let caller_ura = caller_ura.trim();
    if caller_ura.is_empty() {
        None
    } else {
        Some(caller_ura)
    }
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

    #[test]
    fn route_negative_owner_offline_is_route_unavailable_not_ability_absent() {
        let failure = failure(
            "ABILITY_NOT_FOUND",
            "ROUTE_NEGATIVE: namespace.resolve negative for \
             `easynet:///r/localhost/ability/device.dev-a.meta.list_abilities`: \
             NEGATIVE_REASON_NXDOMAIN: owner is not online",
        );

        let status = status_from_remote_failure("remote Invoke", "ignored", Some(&failure));

        assert_eq!(status.code(), Code::Unavailable);
        assert!(status.message().contains("ROUTE_NEGATIVE"));
        assert!(status.message().contains("owner is not online"));
    }

    #[test]
    fn caller_signer_readiness_is_not_downgraded_to_ability_absent() {
        let failure = failure(
            "ABILITY_NOT_FOUND",
            "easynet_runtime_resolve_descriptor_ref: remote invocation requires a caller signer \
             for `easynet:///r/localhost/user/alice`; load or provision that identity in the \
             local key service: self-identity: keyring rejected request: kind=not_found, \
             msg=keyring entry not found: easynet:///r/localhost/user/alice",
        );

        let status = status_from_remote_failure("remote Invoke", "ignored", Some(&failure));

        assert_eq!(status.code(), Code::PermissionDenied);
        assert!(status.message().contains("CALLER_SIGNER_UNAVAILABLE"));
        assert!(status.message().contains("requires a caller signer"));
        assert!(status
            .message()
            .contains("easynet:///r/localhost/user/alice"));
        assert!(
            !status.message().contains("keyring entry not found")
                && !status.message().contains("keyring rejected request")
                && !status.message().contains("self-identity:"),
            "remote failure must not expose keyring implementation detail: {}",
            status.message()
        );
    }
}
