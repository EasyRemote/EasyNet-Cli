//! Typed projection from a remote runtime failure into gRPC transport status.
//!
//! Presence carriers already return a stable SessionFailure. The forwarding
//! daemon must preserve that semantic class instead of flattening every
//! pre-finalization rejection into FAILED_PRECONDITION or UNAVAILABLE.

use tonic::Status;

use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;
use crate::daemon::runtime_failure::{
    canonical_untyped_remote_failure_detail, RuntimeFailureFacts,
};

pub(crate) fn status_from_remote_failure(
    context: &str,
    raw_error: &str,
    failure: Option<&SessionFailure>,
) -> Status {
    let Some(failure) = failure else {
        return Status::unavailable(format!(
            "{context}: {}",
            canonical_untyped_remote_failure_detail(raw_error)
        ));
    };
    let raw_detail = failure.status_detail();
    let facts = RuntimeFailureFacts::new(&failure.code, &raw_detail);
    let detail = facts.canonical_detail();
    let code = facts.grpc_status_code();
    Status::new(code, format!("{context}: {detail}"))
}

pub(crate) fn is_admission_denial_message(message: &str) -> bool {
    RuntimeFailureFacts::is_admission_denial_message(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

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
    fn untyped_remote_failure_does_not_gain_canonical_authority_class() {
        let status = status_from_remote_failure(
            "remote Invoke",
            "AUTHORITY_DENIED: target rejected caller",
            None,
        );

        assert_eq!(status.code(), Code::Unavailable);
        assert!(status.message().contains("REMOTE_FAILURE_UNTYPED"));
        assert!(status.message().contains("AUTHORITY_DENIED"));
    }

    #[test]
    fn untyped_remote_failure_redacts_keyring_implementation_detail() {
        let status = status_from_remote_failure(
            "remote Invoke",
            "self-identity: keyring rejected request: kind=not_found, \
             msg=keyring entry not found: easynet:///r/localhost/user/alice",
            None,
        );

        assert_eq!(status.code(), Code::Unavailable);
        assert!(status.message().contains("REMOTE_FAILURE_UNTYPED"));
        assert!(status.message().contains("custody detail redacted"));
        assert!(
            !status.message().contains("keyring entry not found")
                && !status.message().contains("keyring rejected request")
                && !status.message().contains("self-identity:"),
            "untyped remote failure must not expose custody implementation detail: {}",
            status.message()
        );
    }

    #[test]
    fn typed_owner_offline_is_route_unavailable_not_ability_absent() {
        let failure = failure(
            "DESCRIPTOR_OWNER_OFFLINE",
            "ROUTE_NEGATIVE: namespace.resolve negative for \
             `easynet:///r/localhost/ability/device.dev-a.meta.list_abilities`: \
             NEGATIVE_REASON_NXDOMAIN: owner is not online",
        );

        let status = status_from_remote_failure("remote Invoke", "ignored", Some(&failure));

        assert_eq!(status.code(), Code::Unavailable);
        assert!(
            status
                .message()
                .contains("DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online"),
            "owner-offline route failure must be canonicalized: {}",
            status.message()
        );
        assert!(!status.message().contains("ABILITY_NOT_FOUND"));
    }

    #[test]
    fn route_text_does_not_gain_owner_offline_state() {
        let failure = failure(
            "ABILITY_NOT_FOUND",
            "ROUTE_NEGATIVE: namespace.resolve negative for \
             `easynet:///r/localhost/ability/device.dev-a.meta.list_abilities`: \
             NEGATIVE_REASON_NXDOMAIN: owner is not online",
        );

        let status = status_from_remote_failure("remote Invoke", "ignored", Some(&failure));

        assert_eq!(status.code(), Code::NotFound);
        assert!(status.message().contains("ROUTE_NEGATIVE"));
        assert!(!status.message().contains("DESCRIPTOR_OWNER_OFFLINE"));
    }

    #[test]
    fn typed_caller_signer_readiness_is_not_downgraded_to_ability_absent() {
        let failure = failure(
            "CALLER_SIGNER_UNAVAILABLE",
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

    #[test]
    fn keyring_text_does_not_gain_caller_signer_state() {
        let failure = failure(
            "ABILITY_NOT_FOUND",
            "easynet_runtime_resolve_descriptor_ref: remote invocation requires a caller signer \
             for `easynet:///r/localhost/user/alice`; load or provision that identity in the \
             local key service: self-identity: keyring rejected request: kind=not_found, \
             msg=keyring entry not found: easynet:///r/localhost/user/alice",
        );

        let status = status_from_remote_failure("remote Invoke", "ignored", Some(&failure));

        assert_eq!(status.code(), Code::NotFound);
        assert!(!status.message().contains("CALLER_SIGNER_UNAVAILABLE"));
    }
}
