// EasyNet CLI - daemon Invocation session failure projection
// ========================================
//
// File: src/daemon/invocation/state/session_failure.rs
// Description: Typed failure value object shared by product session wires.
//
// This type intentionally lives outside the axon-pb-gated invocation
// transport. Pending dispatch correlation, local session dispatch, and daemon
// invocation producers all need the same canonical failure projection even
// when the binary is compiled without the Axon protobuf transport feature.

use serde::{Deserialize, Serialize};

/// Canonical terminal failure projection carried across EasyNet session JSON
/// frames. This mirrors the stable fields of Axon `Error` without making the
/// product session wire depend on prost's generated message serde behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub stage: i32,
    pub security_class: i32,
}

impl SessionFailure {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        stage: i32,
        security_class: i32,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            stage,
            security_class,
        }
    }

    pub fn from_reason(reason: impl Into<String>, default_code: &str, retryable: bool) -> Self {
        let message = reason.into();
        let code =
            crate::daemon::execution::mission::failure_codes::FailureCodeClassifier::classify_or_default(
                &message,
                default_code,
            );
        Self::from_code_and_message(code, message, retryable)
    }

    pub fn from_explicit(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        let code =
            crate::daemon::execution::mission::failure_codes::FailureCodeClassifier::normalize_or_default(
                &code.into(),
                "INVOCATION_FAILED",
            );
        Self::from_code_and_message(code, message.into(), retryable)
    }

    /// Render the stable operator/API detail for a terminal session failure.
    ///
    /// The wire already carries typed fields. This helper exists so every
    /// transport projection that still has to return a status message preserves
    /// the canonical code in the same `CODE: message` shape instead of
    /// re-flattening the failure into free text.
    pub fn status_detail(&self) -> String {
        if self.message.trim().is_empty() {
            self.code.clone()
        } else {
            format!("{}: {}", self.code, self.message)
        }
    }

    fn from_code_and_message(code: String, message: String, retryable: bool) -> Self {
        let failure_class =
            crate::daemon::execution::mission::failure_codes::FailureCodeClassifier::classify_error_class(&code);
        Self::new(
            code,
            message,
            retryable,
            failure_class.stage.axon_number(),
            failure_class.security_class.axon_number(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::SessionFailure;

    #[test]
    fn session_failure_wire_requires_retry_and_classification_facts() {
        let legacy = serde_json::json!({
            "code": "TARGET_OFFLINE",
            "message": "target device is offline"
        });

        let error = serde_json::from_value::<SessionFailure>(legacy)
            .expect_err("session failure wire must reject missing typed failure facts");
        let message = error.to_string();
        assert!(
            message.contains("retryable")
                || message.contains("stage")
                || message.contains("security_class"),
            "missing typed failure facts must be surfaced as a schema failure: {message}"
        );
    }

    #[test]
    fn session_failure_wire_rejects_unknown_fields() {
        let legacy = serde_json::json!({
            "code": "TARGET_OFFLINE",
            "message": "target device is offline",
            "retryable": true,
            "stage": 3,
            "security_class": 1,
            "state_code": "legacy"
        });

        let error = serde_json::from_value::<SessionFailure>(legacy)
            .expect_err("session failure wire must reject read-model drift");

        assert!(
            error.to_string().contains("state_code"),
            "decode error should name the noncanonical field: {error}"
        );
    }

    #[test]
    fn session_failure_wire_round_trips_complete_facts() {
        let failure =
            SessionFailure::from_reason("target device is offline", "TARGET_OFFLINE", true);
        let encoded = serde_json::to_value(&failure).expect("session failure serializes");
        assert_eq!(encoded["code"], "TARGET_OFFLINE");
        assert_eq!(encoded["message"], "target device is offline");
        assert_eq!(encoded["retryable"], true);
        assert!(encoded.get("stage").is_some());
        assert!(encoded.get("security_class").is_some());

        let decoded: SessionFailure =
            serde_json::from_value(encoded).expect("complete session failure wire decodes");
        assert_eq!(decoded, failure);
    }
}
