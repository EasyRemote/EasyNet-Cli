// EasyNet CLI — remote desktop domain errors
// ==========================================
//
// File: plugins/remote-desktop/src/errors.rs
// Description: Typed remote desktop errors at session and handler boundaries.

use crate::daemon::plugins::remote_desktop::constants::{
    REASON_CONSENT_RECEIPT_MISMATCH, REASON_CONSENT_RECEIPT_REQUIRED, REASON_INVALID_ARGUMENT,
    REASON_SESSION_CALLER_MISMATCH, REASON_SESSION_EXPIRED, REASON_SESSION_NOT_FOUND,
    REASON_SESSION_STORE_FULL, REASON_SESSION_TERMINAL, REASON_SESSION_TOKEN_MISMATCH,
    REASON_SESSION_TOKEN_REQUIRED,
};
use axon_sdk::invocation::{AxonError, AxonErrorKind, ErrorCode, ErrorStage, SecurityClass};

/// Domain error for remote desktop session access and lifecycle checks.
///
/// What this is NOT: a transport error wrapper. Media/WebRTC/process failures
/// remain projected through session events. This type pins authorization,
/// identity, and state-transition failures before the ability handler boundary
/// converts them to `anyhow` at the ability registry boundary.
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
    #[error("{ability}: session {session_id:?} not found; reason={REASON_SESSION_NOT_FOUND}")]
    SessionNotFound {
        ability: &'static str,
        session_id: String,
    },
    #[error("{ability}: session {session_id:?} lease expired; reason={REASON_SESSION_EXPIRED}")]
    SessionExpired {
        ability: &'static str,
        session_id: String,
    },
    #[error("{ability}: session {session_id:?} is terminal; reason={REASON_SESSION_TERMINAL}")]
    SessionTerminal {
        ability: &'static str,
        session_id: String,
    },
    #[error("{ability}: remote desktop session store is full; reason={REASON_SESSION_STORE_FULL}")]
    SessionStoreFull { ability: &'static str },
    #[error("{ability}: transport epoch {epoch} is not active")]
    TransportEpochMismatch { ability: &'static str, epoch: u64 },
}

pub(in crate::daemon::plugins::remote_desktop) type RemoteDesktopResult<T> =
    std::result::Result<T, RemoteDesktopError>;

impl RemoteDesktopError {
    pub(in crate::daemon::plugins::remote_desktop) fn to_axon(&self) -> AxonError {
        let (kind, code, stage, security_class) = match self {
            Self::InvalidArgument { .. } => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::RequestPayloadInvalid,
                ErrorStage::RequestValidation,
                SecurityClass::Resource,
            ),
            Self::SessionTokenRequired { .. } | Self::ConsentReceiptRequired { .. } => (
                AxonErrorKind::PermissionDenied,
                ErrorCode::AuthorityRequired,
                ErrorStage::AuthorityValidation,
                SecurityClass::Authority,
            ),
            Self::SessionTokenMismatch { .. }
            | Self::SessionCallerMismatch { .. }
            | Self::ConsentReceiptMismatch { .. } => (
                AxonErrorKind::PermissionDenied,
                ErrorCode::AbilityForbidden,
                ErrorStage::AbilityPolicy,
                SecurityClass::Authorization,
            ),
            Self::SessionNotFound { .. } => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::NotFound,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
            Self::SessionExpired { .. } | Self::SessionTerminal { .. } => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::RequestMetadataInvalid,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
            Self::SessionStoreFull { .. } => (
                AxonErrorKind::ResourceExhausted,
                ErrorCode::ResourceExhausted,
                ErrorStage::Quota,
                SecurityClass::Resource,
            ),
            Self::TransportEpochMismatch { .. } => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::RequestMetadataInvalid,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
        };
        AxonError::new(kind)
            .with_code(code)
            .with_stage(stage)
            .with_security_class(security_class)
            .with_message(self.to_string())
    }
}
