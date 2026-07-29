//! Canonical daemon runtime failure classification.
//!
//! Runtime adapters observe failures from multiple carriers: local daemon
//! errors, remote session facts, descriptor resolution, and FFI bridges. The
//! semantic classification must stay in one daemon-owned boundary so callers do
//! not infer product-visible states by parsing unrelated lower-layer messages.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeFailureKind {
    CallerSignerUnavailable,
    DescriptorOwnerOffline,
    RouteUnavailable,
    AdmissionDenied,
    NotFound,
    InvalidRequest,
    ResourceExhausted,
    Timeout,
    TargetUnavailable,
    Internal,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeFailureFacts<'a> {
    code: &'a str,
    detail: &'a str,
}

impl<'a> RuntimeFailureFacts<'a> {
    pub(crate) fn new(code: &'a str, detail: &'a str) -> Self {
        Self { code, detail }
    }

    pub(crate) fn classify(self) -> RuntimeFailureKind {
        let code = self.normalized_code();
        if self.is_descriptor_owner_offline(&code) {
            return RuntimeFailureKind::DescriptorOwnerOffline;
        }
        if self.is_route_unavailable(&code) {
            return RuntimeFailureKind::RouteUnavailable;
        }
        if self.is_caller_signer_unavailable(&code) {
            return RuntimeFailureKind::CallerSignerUnavailable;
        }
        if self.is_admission_denial(&code) {
            return RuntimeFailureKind::AdmissionDenied;
        }
        if matches!(code.as_str(), "NOT_FOUND" | "ABILITY_NOT_FOUND") {
            return RuntimeFailureKind::NotFound;
        }
        if code.starts_with("REQUEST_")
            || matches!(
                code.as_str(),
                "CALLER_URA_MISSING" | "CALLER_URA_INVALID" | "CALLER_NONCE_MISSING"
            )
        {
            return RuntimeFailureKind::InvalidRequest;
        }
        if matches!(
            code.as_str(),
            "QUOTA_EXCEEDED" | "RATE_LIMITED" | "RESOURCE_EXHAUSTED"
        ) {
            return RuntimeFailureKind::ResourceExhausted;
        }
        if code == "UPSTREAM_TIMEOUT" {
            return RuntimeFailureKind::Timeout;
        }
        if matches!(code.as_str(), "UPSTREAM_FAILURE" | "TARGET_OFFLINE") {
            return RuntimeFailureKind::TargetUnavailable;
        }
        if matches!(
            code.as_str(),
            "INTERNAL_ERROR" | "INTERNAL_INVARIANT_VIOLATION"
        ) {
            return RuntimeFailureKind::Internal;
        }
        RuntimeFailureKind::Other
    }

    #[cfg(feature = "axon-pb")]
    pub(crate) fn grpc_status_code(self) -> tonic::Code {
        match self.classify() {
            RuntimeFailureKind::DescriptorOwnerOffline
            | RuntimeFailureKind::RouteUnavailable
            | RuntimeFailureKind::TargetUnavailable => tonic::Code::Unavailable,
            RuntimeFailureKind::CallerSignerUnavailable | RuntimeFailureKind::AdmissionDenied => {
                tonic::Code::PermissionDenied
            }
            RuntimeFailureKind::NotFound => tonic::Code::NotFound,
            RuntimeFailureKind::InvalidRequest => tonic::Code::InvalidArgument,
            RuntimeFailureKind::ResourceExhausted => tonic::Code::ResourceExhausted,
            RuntimeFailureKind::Timeout => tonic::Code::DeadlineExceeded,
            RuntimeFailureKind::Internal => tonic::Code::Internal,
            RuntimeFailureKind::Other => tonic::Code::FailedPrecondition,
        }
    }

    pub(crate) fn canonical_detail(self) -> String {
        let detail = self.detail.trim();
        match self.classify() {
            RuntimeFailureKind::CallerSignerUnavailable => {
                canonical_caller_signer_unavailable_detail(detail)
            }
            RuntimeFailureKind::DescriptorOwnerOffline => {
                "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online".to_string()
            }
            _ => detail.to_string(),
        }
    }

    pub(crate) fn is_admission_denial_message<'b>(message: &'b str) -> bool {
        canonical_code_from_message_prefix(message).is_some_and(|code| {
            RuntimeFailureFacts::new(code, "").classify() == RuntimeFailureKind::AdmissionDenied
        })
    }

    pub(crate) fn is_caller_signer_unavailable_message<'b>(message: &'b str) -> bool {
        canonical_code_from_message_prefix(message).is_some_and(|code| {
            RuntimeFailureFacts::new(code, "").classify()
                == RuntimeFailureKind::CallerSignerUnavailable
        })
    }

    #[cfg(feature = "axon-pb")]
    pub(crate) fn is_descriptor_owner_offline_status<'b>(
        code: tonic::Code,
        message: &'b str,
    ) -> bool {
        code == tonic::Code::Unavailable
            && canonical_code_from_message_prefix(message).is_some_and(|code| {
                RuntimeFailureFacts::new(code, "").classify()
                    == RuntimeFailureKind::DescriptorOwnerOffline
            })
    }

    fn normalized_code(self) -> String {
        self.code.trim().to_ascii_uppercase()
    }

    fn is_route_unavailable(self, code: &str) -> bool {
        code == "ROUTE_UNAVAILABLE"
            || code == "RUNTIME_ROUTE_UNAVAILABLE"
            || code == "ROUTE_NEGATIVE"
    }

    fn is_caller_signer_unavailable(self, code: &str) -> bool {
        code == "CALLER_SIGNER_UNAVAILABLE"
    }

    fn is_descriptor_owner_offline(self, code: &str) -> bool {
        code == "DESCRIPTOR_OWNER_OFFLINE"
    }

    fn is_admission_denial(self, code: &str) -> bool {
        code == "PERMISSION_DENIED"
            || code.starts_with("AUTHORITY_")
            || code.starts_with("POLICY_")
            || code.starts_with("SIGNATURE_")
            || (code.starts_with("CALLER_")
                && !matches!(
                    code,
                    "CALLER_URA_MISSING" | "CALLER_URA_INVALID" | "CALLER_NONCE_MISSING"
                ))
            || matches!(
                code,
                "ABILITY_FORBIDDEN" | "ABILITY_ROLE_RESTRICTED" | "ABILITY_REALM_RESTRICTED"
            )
    }
}

fn canonical_code_from_message_prefix(message: &str) -> Option<&str> {
    let (prefix, _) = message.trim().split_once(':')?;
    let code = prefix.trim();
    if code.is_empty()
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
    {
        return None;
    }
    Some(code)
}

pub(crate) fn canonical_untyped_remote_failure_detail(raw_error: &str) -> String {
    let detail = raw_error.trim();
    if detail.is_empty() {
        return "REMOTE_FAILURE_UNTYPED: remote failure omitted typed failure facts".to_string();
    }
    if contains_custody_implementation_detail(detail) {
        return "REMOTE_FAILURE_UNTYPED: remote failure omitted typed failure facts; \
                custody detail redacted"
            .to_string();
    }
    format!(
        "REMOTE_FAILURE_UNTYPED: remote failure omitted typed failure facts; diagnostic={}",
        bounded_diagnostic(detail)
    )
}

fn contains_custody_implementation_detail(detail: &str) -> bool {
    let detail = detail.to_ascii_uppercase();
    detail.contains("KEYRING ENTRY NOT FOUND")
        || detail.contains("KEYRING REJECTED REQUEST")
        || detail.contains("SELF-IDENTITY:")
}

fn bounded_diagnostic(detail: &str) -> String {
    const MAX_DIAGNOSTIC_CHARS: usize = 256;
    let mut chars = detail.chars();
    let clipped: String = chars.by_ref().take(MAX_DIAGNOSTIC_CHARS).collect();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
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

    #[test]
    fn classifies_typed_caller_signer_unavailable() {
        let facts = RuntimeFailureFacts::new(
            "CALLER_SIGNER_UNAVAILABLE",
            "easynet_runtime_resolve_descriptor_ref: remote invocation requires a caller signer \
             for `easynet:///r/localhost/user/alice`; load or provision that identity in the \
             local key service: self-identity: keyring rejected request: kind=not_found, \
             msg=keyring entry not found: easynet:///r/localhost/user/alice",
        );

        assert_eq!(
            facts.classify(),
            RuntimeFailureKind::CallerSignerUnavailable
        );
        let detail = facts.canonical_detail();
        assert!(detail.contains("CALLER_SIGNER_UNAVAILABLE"));
        assert!(detail.contains("easynet:///r/localhost/user/alice"));
        assert!(!detail.contains("keyring entry not found"));
        assert!(!detail.contains("self-identity:"));
    }

    #[test]
    fn untyped_keyring_detail_does_not_gain_caller_signer_state() {
        let facts = RuntimeFailureFacts::new(
            "ABILITY_NOT_FOUND",
            "easynet_runtime_resolve_descriptor_ref: remote invocation requires a caller signer \
             for `easynet:///r/localhost/user/alice`; load or provision that identity in the \
             local key service: self-identity: keyring rejected request: kind=not_found, \
             msg=keyring entry not found: easynet:///r/localhost/user/alice",
        );

        assert_eq!(facts.classify(), RuntimeFailureKind::NotFound);
        assert!(!facts
            .canonical_detail()
            .contains("CALLER_SIGNER_UNAVAILABLE"));
    }

    #[test]
    fn classifies_typed_owner_offline_before_not_found() {
        let facts = RuntimeFailureFacts::new(
            "DESCRIPTOR_OWNER_OFFLINE",
            "ROUTE_NEGATIVE: namespace.resolve negative: NEGATIVE_REASON_NXDOMAIN: owner is not online",
        );

        assert_eq!(facts.classify(), RuntimeFailureKind::DescriptorOwnerOffline);
        assert_eq!(
            facts.canonical_detail(),
            "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online"
        );
    }

    #[test]
    fn route_text_does_not_gain_descriptor_owner_offline_state() {
        let facts = RuntimeFailureFacts::new(
            "ABILITY_NOT_FOUND",
            "ROUTE_NEGATIVE: namespace.resolve negative: NEGATIVE_REASON_NXDOMAIN: owner is not online",
        );

        assert_eq!(facts.classify(), RuntimeFailureKind::NotFound);
        assert!(!facts
            .canonical_detail()
            .contains("DESCRIPTOR_OWNER_OFFLINE"));
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn status_helpers_require_canonical_code_prefix() {
        assert!(RuntimeFailureFacts::is_caller_signer_unavailable_message(
            "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer"
        ));
        assert!(!RuntimeFailureFacts::is_caller_signer_unavailable_message(
            "remote invocation requires a caller signer"
        ));
        assert!(RuntimeFailureFacts::is_descriptor_owner_offline_status(
            tonic::Code::Unavailable,
            "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online"
        ));
        assert!(!RuntimeFailureFacts::is_descriptor_owner_offline_status(
            tonic::Code::Unavailable,
            "ROUTE_NEGATIVE: namespace.resolve negative: NEGATIVE_REASON_NXDOMAIN: owner is not online"
        ));
    }

    #[test]
    fn untyped_remote_failure_redacts_custody_storage_detail() {
        let detail = canonical_untyped_remote_failure_detail(
            "self-identity: keyring rejected request: kind=not_found, \
             msg=keyring entry not found: easynet:///r/localhost/user/alice",
        );

        assert!(detail.contains("custody detail redacted"));
        assert!(!detail.contains("keyring entry not found"));
        assert!(!detail.contains("self-identity:"));
    }
}
