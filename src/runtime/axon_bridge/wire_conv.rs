//! 1:1 conversions from the proto-generated wire types
//! (`crate::pb::axon::v1::*`) to the Axon SDK pure-Rust types
//! (`easynet_axon::invocation::*`).
//!
//! Why this lives in CLI and not in Axon SDK
//! ----------------------------------------
//! Axon SDK is **proto-agnostic by design** — `sdk/rust/Cargo.toml`
//! does not depend on `prost` or `tonic` (only `core/runtime-rs`
//! does the proto compile). Every wire-receiving consumer therefore
//! has to bridge "proto type at the wire boundary" → "SDK type at
//! the admission boundary" in its own crate. The orphan rule
//! permits this here because the proto types live in CLI's local
//! `pb::axon::v1` module.
//!
//! The **only** intelligent transformation that belongs in Axon is
//! seven-tuple reassembly from the proto's deliberately-split
//! shape (`Envelope` carries 4 fields, `EnvelopeOpen` carries
//! `ability` + `args`) — that's `InvocationEnvelope::from_wire_parts`
//! and it lives in Axon (`sdk/rust/src/invocation/axiom.rs:203`).
//! Everything in this file is pure field copy with one enum
//! conversion (UraProfile) — no semantic risk, but Axon SDK
//! intentionally doesn't import prost types just to ship these
//! ten-line conversions. The cross-SDK consistency check is that
//! `InvocationEnvelope::from_wire_parts` followed by
//! `canonical_invocation_bytes` produces the **same bytes** any
//! other SDK's bridge produces — that's pinned in Axon's
//! `from_wire_parts_*` tests.

use easynet_axon::invocation::error::{AxonError, AxonErrorKind};
use easynet_axon::invocation::{
    AgentIdentity, CallerSignature, CausalContext, ReceiptRef, SubjectIdentity, UraProfile,
};

use crate::pb::axon::v1 as pb;

// ── AgentIdentity ───────────────────────────────────────────────────

/// Map the proto `profile` string to the SDK's enum. Unknown
/// profile strings map to `EasynetStrictV2` (the spec default)
/// rather than rejecting, so an older wire client that emits
/// `profile=""` still admits — consistent with how the rest of CLI
/// admission has always behaved at this boundary.
fn parse_profile_lenient(profile: &str) -> UraProfile {
    if profile.is_empty() {
        UraProfile::EasynetStrictV2
    } else {
        UraProfile::parse(profile).unwrap_or(UraProfile::EasynetStrictV2)
    }
}

impl From<pb::AgentIdentity> for AgentIdentity {
    fn from(wire: pb::AgentIdentity) -> Self {
        AgentIdentity::new(wire.ura, parse_profile_lenient(&wire.profile))
    }
}

impl From<pb::SubjectIdentity> for SubjectIdentity {
    fn from(wire: pb::SubjectIdentity) -> Self {
        SubjectIdentity::new(wire.ura, parse_profile_lenient(&wire.profile))
    }
}

// ── CallerSignature ─────────────────────────────────────────────────

impl From<pb::CallerSignature> for CallerSignature {
    fn from(wire: pb::CallerSignature) -> Self {
        CallerSignature {
            algorithm: wire.algorithm,
            signature: wire.signature,
            key_id_hint: wire.key_id_hint,
        }
    }
}

// ── CausalContext ───────────────────────────────────────────────────

/// 32-byte big-endian receipt-hash extractor with length check.
fn try_receipt_hash(bytes: Vec<u8>) -> Result<[u8; 32], AxonError> {
    let len = bytes.len();
    bytes.try_into().map_err(|_| {
        AxonError::invalid_argument(format!("receipt_hash must be 32 bytes, got {len}"))
    })
}

fn try_receipt_ref(rr: pb::ReceiptRef) -> Result<ReceiptRef, AxonError> {
    Ok(ReceiptRef {
        receipt_hash: try_receipt_hash(rr.receipt_hash)?,
        receipt_ura: rr.receipt_ura,
    })
}

/// Convert the proto `CausalContext` oneof into the SDK enum.
/// Returns `AxonError::invalid_argument` for malformed payloads
/// (wrong byte lengths, missing required Receipt fields). An
/// absent `causal_context` field on the wire (`None` at the proto
/// level) maps to `CausalContext::None`, matching RFC 001's
/// "root invocation" default.
pub fn causal_context_from_wire(
    wire: Option<pb::CausalContext>,
) -> Result<CausalContext, AxonError> {
    let Some(cc) = wire else {
        return Ok(CausalContext::None);
    };
    let Some(form) = cc.form else {
        return Ok(CausalContext::None);
    };
    use pb::causal_context::Form;
    match form {
        Form::None(_) => Ok(CausalContext::None),
        Form::Scalar(rr) => Ok(CausalContext::Scalar(try_receipt_ref(rr)?)),
        Form::List(rl) => {
            let mut prior = Vec::with_capacity(rl.prior.len());
            for rr in rl.prior {
                prior.push(try_receipt_ref(rr)?);
            }
            Ok(CausalContext::List(prior))
        }
        Form::Merkle(mr) => Ok(CausalContext::Merkle {
            root: try_receipt_hash(mr.root)?,
            proof_ura: mr.proof_ura,
        }),
    }
}

// ── Envelope helpers ────────────────────────────────────────────────

/// Extract the canonical 16-byte invocation nonce from a wire
/// envelope's `invocation_nonce` field. Returns `invalid_argument`
/// on length mismatch — admission expects exactly 16 bytes per
/// RFC 001 §4.1.
pub fn try_invocation_nonce(bytes: Vec<u8>) -> Result<[u8; 16], AxonError> {
    let len = bytes.len();
    bytes.try_into().map_err(|_| {
        AxonError::new(AxonErrorKind::InvalidArgument)
            .with_reason(format!("invocation_nonce must be 16 bytes, got {len}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_identity_round_trips_strict_v2_profile() {
        let wire = pb::AgentIdentity {
            ura: "easynet:///r/t/agent/u.x".to_string(),
            profile: "easynet-strict-v2".to_string(),
        };
        let sdk: AgentIdentity = wire.into();
        assert_eq!(sdk.ura, "easynet:///r/t/agent/u.x");
        assert_eq!(sdk.profile, UraProfile::EasynetStrictV2);
    }

    #[test]
    fn agent_identity_empty_profile_defaults_to_strict() {
        // Wire-leniency contract: pre-RFC-001 clients sometimes emit
        // empty profile; admission accepts and defaults to the spec
        // current profile rather than rejecting the inbound call.
        let wire = pb::AgentIdentity {
            ura: "easynet:///r/t/device/d".to_string(),
            profile: String::new(),
        };
        let sdk: AgentIdentity = wire.into();
        assert_eq!(sdk.profile, UraProfile::EasynetStrictV2);
    }

    #[test]
    fn caller_signature_field_for_field_copy() {
        let wire = pb::CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: vec![0u8; 64],
            key_id_hint: "k1".to_string(),
        };
        let sdk: CallerSignature = wire.into();
        assert_eq!(sdk.algorithm, "ed25519");
        assert_eq!(sdk.signature.len(), 64);
        assert_eq!(sdk.key_id_hint, "k1");
    }

    #[test]
    fn causal_context_absent_maps_to_none() {
        assert!(matches!(
            causal_context_from_wire(None).unwrap(),
            CausalContext::None
        ));
    }

    #[test]
    fn causal_context_scalar_carries_receipt_ref() {
        let wire = pb::CausalContext {
            form: Some(pb::causal_context::Form::Scalar(pb::ReceiptRef {
                receipt_hash: vec![0xAB; 32],
                receipt_ura: "easynet:///r/t/resource/r1".to_string(),
            })),
        };
        let sdk = causal_context_from_wire(Some(wire)).unwrap();
        match sdk {
            CausalContext::Scalar(rr) => {
                assert_eq!(rr.receipt_hash, [0xAB; 32]);
                assert_eq!(rr.receipt_ura, "easynet:///r/t/resource/r1");
            }
            other => panic!("expected Scalar, got {other:?}"),
        }
    }

    #[test]
    fn causal_context_scalar_rejects_short_hash() {
        let wire = pb::CausalContext {
            form: Some(pb::causal_context::Form::Scalar(pb::ReceiptRef {
                receipt_hash: vec![0xAB; 31], // one byte short
                receipt_ura: "u".to_string(),
            })),
        };
        let err = causal_context_from_wire(Some(wire)).unwrap_err();
        assert!(err.to_string().contains("32 bytes"));
    }

    #[test]
    fn try_invocation_nonce_accepts_16_rejects_others() {
        assert_eq!(try_invocation_nonce(vec![0; 16]).unwrap(), [0u8; 16]);
        let short = try_invocation_nonce(vec![0; 15]).unwrap_err();
        assert!(short.to_string().contains("16 bytes"));
        let long = try_invocation_nonce(vec![0; 17]).unwrap_err();
        assert!(long.to_string().contains("16 bytes"));
    }
}
