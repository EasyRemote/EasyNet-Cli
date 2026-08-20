//! Typed browser plugin failures projected onto canonical Axon errors.
//! ==================================================================
//!
//! File: plugins/browser/src/errors.rs
//! Description: Browser-domain failures and their canonical Axon projection.
//!
//! Protocol Responsibility:
//! - Preserve reason, stage, security class, and retry semantics at the plugin
//!   boundary without defining a second wire error model.
//!
//! Implementation Approach:
//! - Keep domain variants typed and map them once into `AxonError`.
//!
//! Usage Contract:
//! - Handlers return these errors through the plugin contribution adapter.
//!
//! Architectural Position:
//! - Browser plugin domain-to-Axon anti-corruption boundary.

use axon_sdk::invocation::{AxonError, AxonErrorKind, ErrorCode, ErrorStage, SecurityClass};

use super::constants::*;

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("{ability}: {detail}; reason={REASON_INVALID_ARGUMENT}")]
    InvalidArgument {
        ability: &'static str,
        detail: String,
    },
    #[error(
        "{ability}: invocation subject {actual:?} must equal browser session {expected:?}; reason={REASON_SUBJECT_MISMATCH}"
    )]
    SubjectMismatch {
        ability: &'static str,
        expected: String,
        actual: String,
    },
    #[error(
        "{ability}: caller {actual:?} does not own browser session created by {expected:?}; reason={REASON_CALLER_MISMATCH}"
    )]
    CallerMismatch {
        ability: &'static str,
        expected: String,
        actual: String,
    },
    #[error(
        "{ability}: browser session {session_ura:?} not found; reason={REASON_SESSION_NOT_FOUND}"
    )]
    SessionNotFound {
        ability: &'static str,
        session_ura: String,
    },
    #[error(
        "{ability}: browser session {session_ura:?} is terminal; reason={REASON_SESSION_TERMINAL}"
    )]
    SessionTerminal {
        ability: &'static str,
        session_ura: String,
    },
    #[error("{ability}: browser session store is full; reason={REASON_SESSION_STORE_FULL}")]
    SessionStoreFull { ability: &'static str },
    #[error(
        "{ability}: session already has an Axon CDP attachment; reason={REASON_ATTACHMENT_ACTIVE}"
    )]
    AttachmentActive { ability: &'static str },
    #[error("{ability}: session already has a viewport capture; reason={REASON_CAPTURE_ACTIVE}")]
    CaptureActive { ability: &'static str },
    #[error("{ability}: CDP command {method:?} is denied; reason={REASON_CDP_POLICY}")]
    CdpPolicy {
        ability: &'static str,
        method: String,
    },
    #[error("{ability}: Chrome/CDP unavailable: {detail}; reason={REASON_CDP_UNAVAILABLE}")]
    Unavailable {
        ability: &'static str,
        detail: String,
    },
    #[error("{ability}: CDP operation failed: {detail}")]
    Cdp {
        ability: &'static str,
        detail: String,
    },
}

pub type BrowserResult<T> = std::result::Result<T, BrowserError>;

impl BrowserError {
    pub fn to_axon(&self) -> AxonError {
        let (kind, code, stage, security_class) = match self {
            Self::InvalidArgument { .. } => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::RequestPayloadInvalid,
                ErrorStage::RequestValidation,
                SecurityClass::Resource,
            ),
            Self::SubjectMismatch { .. } | Self::CallerMismatch { .. } | Self::CdpPolicy { .. } => {
                (
                    AxonErrorKind::PermissionDenied,
                    ErrorCode::AbilityForbidden,
                    ErrorStage::AbilityPolicy,
                    SecurityClass::Authorization,
                )
            }
            Self::SessionNotFound { .. } => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::NotFound,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
            Self::SessionTerminal { .. } => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::RequestMetadataInvalid,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
            Self::SessionStoreFull { .. }
            | Self::AttachmentActive { .. }
            | Self::CaptureActive { .. } => (
                AxonErrorKind::ResourceExhausted,
                ErrorCode::ResourceExhausted,
                ErrorStage::Quota,
                SecurityClass::Resource,
            ),
            Self::Unavailable { .. } => (
                AxonErrorKind::Unavailable,
                ErrorCode::ExecutionFailed,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
            Self::Cdp { .. } => (
                AxonErrorKind::Internal,
                ErrorCode::ExecutionFailed,
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
