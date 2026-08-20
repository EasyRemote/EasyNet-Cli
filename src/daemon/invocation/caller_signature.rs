//! Canonical construction of Axon caller signatures on EasyNet wire envelopes.
//!
//! The daemon has exactly one wire-level convention for
//! `CallerSignature.key_id_hint`: it carries the signing public key projection
//! as base64. Verifiers still resolve that key independently through the realm
//! trust anchor; the hint only disambiguates 1:N principal keys and prevents
//! products from inventing per-call signature dialects.

use axon_sdk::pb::axon::v1::CallerSignature;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use crate::daemon::identity::self_identity::{CanonicalSigner, SelfIdentityError};

pub(crate) async fn sign_canonical_caller_signature(
    signer: &dyn CanonicalSigner,
    canonical_bytes: &[u8],
) -> Result<CallerSignature, SelfIdentityError> {
    let public_key = signer.signing_public_key()?;
    let signature = signer.sign_canonical(canonical_bytes).await?;
    Ok(CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature.to_bytes().to_vec(),
        key_id_hint: BASE64_STANDARD.encode(public_key.to_bytes()),
    })
}
