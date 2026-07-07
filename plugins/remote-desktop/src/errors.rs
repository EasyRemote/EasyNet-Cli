// EasyNet CLI — remote desktop domain errors
// ==========================================
//
// File: plugins/remote-desktop/src/errors.rs
// Description: Typed remote desktop errors at session and handler boundaries.

use crate::daemon::plugins::remote_desktop::constants::{
    REASON_CONSENT_RECEIPT_MISMATCH, REASON_CONSENT_RECEIPT_REQUIRED, REASON_INVALID_ARGUMENT,
    REASON_SESSION_CALLER_MISMATCH, REASON_SESSION_TOKEN_MISMATCH, REASON_SESSION_TOKEN_REQUIRED,
};

/// Domain error for remote desktop session access and lifecycle checks.
///
/// What this is NOT: a transport error wrapper. Media/WebRTC/process failures
/// remain projected through session events. This type pins authorization,
/// identity, and state-transition failures before the ability handler boundary
/// converts them to `anyhow` for legacy registry compatibility.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon::plugins::remote_desktop) enum RemoteDesktopError {
    #[error("{ability}: {detail}; reason={REASON_INVALID_ARGUMENT}")]
    InvalidArgument {
        ability: &'static str,
        detail: String,
    },
    #[error("{ability}: `session_token` is required; reason={REASON_SESSION_TOKEN_REQUIRED}")]
    SessionTokenRequired { ability: &'static str },
    #[error(
        "{ability}: session_token does not match session {session_id:?}; reason={REASON_SESSION_TOKEN_MISMATCH}"
    )]
    SessionTokenMismatch {
        ability: &'static str,
        session_id: String,
    },
    #[error(
        "{ability}: caller {actual:?} does not match session creator {expected:?}; reason={REASON_SESSION_CALLER_MISMATCH}"
    )]
    SessionCallerMismatch {
        ability: &'static str,
        expected: String,
        actual: String,
    },
    #[error(
        "{ability}: consent receipt is required for session {session_id:?}; reason={REASON_CONSENT_RECEIPT_REQUIRED}"
    )]
    ConsentReceiptRequired {
        ability: &'static str,
        session_id: String,
    },
    #[error(
        "{ability}: causal consent receipt does not match session approval receipt {expected:?}; reason={REASON_CONSENT_RECEIPT_MISMATCH}"
    )]
    ConsentReceiptMismatch {
        ability: &'static str,
        expected: String,
    },
}

pub(in crate::daemon::plugins::remote_desktop) type RemoteDesktopResult<T> =
    std::result::Result<T, RemoteDesktopError>;
