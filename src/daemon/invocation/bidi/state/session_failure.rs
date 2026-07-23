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
pub struct SessionFailure {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub stage: i32,
    #[serde(default)]
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
