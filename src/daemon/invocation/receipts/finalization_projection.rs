//! EasyNet CLI — receipt finalization projection
//! =============================================
//!
//! File: src/daemon/invocation/receipts/finalization_projection.rs
//! Description: Daemon-local adapter around Axon's admission/terminal
//!              finalization verifier.
//!
//! Protocol Responsibility:
//! - Delegate all receipt wire canonicalization, signature verification, and
//!   finalization checkpoint rules to Axon.
//! - Keep daemon callers from re-implementing the admission + terminal proof
//!   pipeline.
//!
//! Implementation Approach:
//! - Decode one wire receipt into Axon's signed receipt type.
//! - Verify stage-specific receipt state before exposing incremental
//!   checkpoints.
//! - Verify an admission/terminal pair through Axon's
//!   FinalizationCheckpointVerifier.
//!
//! Usage Contract:
//! - Callers may project verified checkpoints into daemon DTOs, but must not
//!   infer complete-chain proof from this two-checkpoint verifier.
//! - Forwarded-invocation callers that need input/output binding checks must
//!   add those checks above this helper.
//!
//! Architectural Position:
//! - Daemon receipt adapter. Axon remains the owner of cryptographic proof
//!   semantics; product modules own only presentation-specific projections.

use axon_sdk::invocation::{
    FinalizationCheckpointVerifier, InvocationState, KeyResolver, SignedInvocationReceipt,
    VerifiedFinalizationCheckpoints,
};
use axon_sdk::pb::axon::v1::InvocationReceipt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ReceiptCheckpointStage {
    Admission,
    Terminal,
}

impl ReceiptCheckpointStage {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum FinalizationProjectionError {
    #[error("{stage} receipt wire proof is malformed: {source}")]
    WireMalformed {
        stage: &'static str,
        source: axon_sdk::invocation::AxonError,
    },
    #[error("{stage} receipt signature is invalid: {source}")]
    SignatureInvalid {
        stage: &'static str,
        source: axon_sdk::invocation::AxonError,
    },
    #[error("admission checkpoint did not verify as Admitted")]
    AdmissionState,
    #[error("terminal checkpoint did not verify as terminal")]
    TerminalState,
    #[error("terminal receipt arrived before a verified admission receipt")]
    TerminalBeforeAdmission,
    #[error("finalization cryptographic verification failed: {source}")]
    Finalization {
        source: axon_sdk::invocation::AxonError,
    },
}

pub(crate) fn verify_wire_checkpoint(
    receipt: InvocationReceipt,
    resolver: &dyn KeyResolver,
    stage: ReceiptCheckpointStage,
) -> Result<SignedInvocationReceipt, FinalizationProjectionError> {
    let stage_label = stage.as_str();
    let canonical =
        axon_sdk::invocation::wire::try_receipt_from_wire(receipt).map_err(|source| {
            FinalizationProjectionError::WireMalformed {
                stage: stage_label,
                source,
            }
        })?;
    canonical
        .verify(resolver)
        .map_err(|source| FinalizationProjectionError::SignatureInvalid {
            stage: stage_label,
            source,
        })
}

pub(crate) fn verify_admission_checkpoint(
    receipt: InvocationReceipt,
    resolver: &dyn KeyResolver,
) -> Result<SignedInvocationReceipt, FinalizationProjectionError> {
    let signed = verify_wire_checkpoint(receipt, resolver, ReceiptCheckpointStage::Admission)?;
    if signed.state() != InvocationState::Admitted {
        return Err(FinalizationProjectionError::AdmissionState);
    }
    Ok(signed)
}

pub(crate) fn verify_terminal_checkpoint(
    receipt: InvocationReceipt,
    resolver: &dyn KeyResolver,
) -> Result<SignedInvocationReceipt, FinalizationProjectionError> {
    let signed = verify_wire_checkpoint(receipt, resolver, ReceiptCheckpointStage::Terminal)?;
    if !signed.state().is_terminal() {
        return Err(FinalizationProjectionError::TerminalState);
    }
    Ok(signed)
}

pub(crate) fn verify_signed_finalization_checkpoints(
    admission: &SignedInvocationReceipt,
    terminal: &SignedInvocationReceipt,
    resolver: &dyn KeyResolver,
) -> Result<VerifiedFinalizationCheckpoints, FinalizationProjectionError> {
    FinalizationCheckpointVerifier::new(resolver)
        .verify(admission, terminal)
        .map_err(|source| FinalizationProjectionError::Finalization { source })
}

pub(crate) fn verify_wire_finalization_checkpoints(
    admission: InvocationReceipt,
    terminal: InvocationReceipt,
    resolver: &dyn KeyResolver,
) -> Result<VerifiedFinalizationCheckpoints, FinalizationProjectionError> {
    let admission = verify_admission_checkpoint(admission, resolver)?;
    let terminal = verify_terminal_checkpoint(terminal, resolver)?;
    verify_signed_finalization_checkpoints(&admission, &terminal, resolver)
}
