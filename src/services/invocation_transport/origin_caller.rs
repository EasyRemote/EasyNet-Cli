// EasyNet CLI — invoke_remote inner user-caller pass-through
// ===========================================================
//
// File: src/services/invocation_transport/origin_caller.rs
// Description: Parse the `x-easynet-origin-caller` metadata item that
//              a hub/backend attaches to an `<self>.invoke_remote`
//              Request when the inner ability was triggered by a
//              browser-signed user. The receiving daemon uses it to
//              reconstruct the INNER invocation envelope with the real
//              user as caller and dispatch via
//              `invoke_externally_signed_async` (cryptographic
//              admission against the user's registered pubkey) instead
//              of the `_system` trust-domain fallback.
//
// Why this exists
// ---------------
// The outer `<self>.invoke_remote` frame-0 envelope is signed by the
// backend over the AXIOM-7 tuple (caller=backend/hub, callee=device,
// subject=device, ability="<self>.invoke_remote"). That is correct and
// stays as-is. The USER's identity belongs to the inner ability call
// (e.g. `remote_desktop.create_session`), which previously had no wire
// carrier — so the inner dispatch defaulted to a fabricated `_system`
// caller and fail-closed abilities (remote desktop consent) rejected.
//
// This module is the additive, optional carrier: when present and the
// inner signature verifies, the inner ability sees the real user as
// `EnvelopeContext.caller`. When absent (older backend) the caller
// falls back to the existing path. A forged value fails signature
// verification in `invoke_externally_signed_async` → falls closed.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;

use base64::Engine as _;
use easynet_axon::invocation::{
    AgentIdentity, CallerSignature, CausalContext, InvocationEnvelope, SubjectIdentity, UraProfile,
};
use serde::{Deserialize, Serialize};

use crate::runtime::axon_bridge::dispatch_shim::WireDispatch;

/// LEGACY metadata key carrying the inner user envelope + browser
/// signature. The canonical carrier is the typed
/// [`OriginCallerClaim`] field on `InvokeRemoteUp::Request` /
/// `SessionDispatch::Dispatch`; this key is read as a fallback during
/// the rolling upgrade and will be removed once the fleet dual-writes
/// the field.
pub const ORIGIN_CALLER_METADATA_KEY: &str = "x-easynet-origin-caller";

const ED25519_ALGORITHM: &str = "ed25519";

/// Typed wire shape of the origin-caller claim. Travels as a
/// first-class `origin_caller` field on `InvokeRemoteUp::Request`
/// and `SessionDispatch::Dispatch` (invocation-unity §22.2: security
/// material rides typed fields, not raw metadata strings). Base64
/// fields are decoded at validation time; an undecodable field makes
/// the whole claim invalid (caller fails closed, never silently
/// mis-binds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginCallerClaim {
    /// Canonical user URA that signed the inner invocation.
    pub caller_ura: String,
    /// The AXIOM `ability` field the browser signed over — the PUBLIC
    /// ability name (e.g. `remote_desktop.create_session`), NOT the
    /// daemon-local dispatch key. The device must rebuild the canonical
    /// bytes with this exact string or the signature won't verify.
    pub ability: String,
    /// Base64 ed25519 signature over the INNER canonical bytes.
    pub signature_b64: String,
    /// Base64 32-byte raw ed25519 verifying key the device resolves
    /// the signature against (key_id_hint for the KeyResolver).
    pub signer_pubkey_b64: String,
    /// Base64 16-byte inner invocation nonce.
    pub nonce_b64: String,
}

/// Decoded, validated origin-caller authorization.
#[derive(Debug, Clone)]
pub struct OriginCaller {
    pub caller_ura: String,
    /// Public ability name the browser signed (canonical `ability` field).
    pub ability: String,
    pub signature: Vec<u8>,
    pub signer_pubkey: Vec<u8>,
    pub nonce: [u8; 16],
}

impl OriginCaller {
    /// Resolve the origin-caller authority for one dispatch: prefer
    /// the typed first-class `claim` field; fall back to the legacy
    /// metadata item during the rolling upgrade. Returns `None` when
    /// neither is present (the common, non-user path). Returns `Err`
    /// when a claim IS present but malformed — a present-but-broken
    /// authority must not silently degrade to `_system`.
    pub fn resolve(
        claim: Option<&OriginCallerClaim>,
        metadata: &HashMap<String, String>,
    ) -> anyhow::Result<Option<Self>> {
        if let Some(claim) = claim {
            return Self::from_claim(claim.clone()).map(Some);
        }
        Self::from_metadata(metadata)
    }

    /// Extract + decode the origin-caller item from the LEGACY
    /// invoke_remote metadata. Kept for one release while the fleet
    /// upgrades to the typed `origin_caller` wire field.
    pub fn from_metadata(metadata: &HashMap<String, String>) -> anyhow::Result<Option<Self>> {
        let Some(raw) = metadata.get(ORIGIN_CALLER_METADATA_KEY) else {
            return Ok(None);
        };
        let claim: OriginCallerClaim = serde_json::from_str(raw)
            .map_err(|e| anyhow::anyhow!("{ORIGIN_CALLER_METADATA_KEY}: invalid JSON: {e}"))?;
        Self::from_claim(claim).map(Some)
    }

    /// Validate + decode a typed claim into the dispatchable form.
    pub fn from_claim(wire: OriginCallerClaim) -> anyhow::Result<Self> {
        if wire.caller_ura.trim().is_empty() {
            anyhow::bail!("{ORIGIN_CALLER_METADATA_KEY}: empty caller_ura");
        }
        if wire.ability.trim().is_empty() {
            anyhow::bail!("{ORIGIN_CALLER_METADATA_KEY}: empty ability");
        }
        let b64 = base64::engine::general_purpose::STANDARD;
        let signature = b64
            .decode(wire.signature_b64.trim())
            .map_err(|e| anyhow::anyhow!("{ORIGIN_CALLER_METADATA_KEY}: bad signature_b64: {e}"))?;
        let signer_pubkey = b64.decode(wire.signer_pubkey_b64.trim()).map_err(|e| {
            anyhow::anyhow!("{ORIGIN_CALLER_METADATA_KEY}: bad signer_pubkey_b64: {e}")
        })?;
        let nonce_vec = b64
            .decode(wire.nonce_b64.trim())
            .map_err(|e| anyhow::anyhow!("{ORIGIN_CALLER_METADATA_KEY}: bad nonce_b64: {e}"))?;
        let nonce: [u8; 16] = nonce_vec.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!(
                "{ORIGIN_CALLER_METADATA_KEY}: nonce must be 16 bytes, got {}",
                nonce_vec.len()
            )
        })?;
        Ok(Self {
            caller_ura: wire.caller_ura,
            ability: wire.ability,
            signature,
            signer_pubkey,
            nonce,
        })
    }

    /// The PUBLIC ability name the browser signed (the canonical
    /// `ability` field). The dispatcher compares this against the
    /// hub-addressed dispatch key to decide whether a dispatch-key
    /// override is needed for agent-owned abilities.
    pub fn public_ability(&self) -> &str {
        &self.ability
    }

    /// Build the inner `WireDispatch` (envelope + signature + payload)
    /// for `invoke_externally_signed_async`. The envelope MUST reproduce
    /// the exact AXIOM-7 tuple the browser signed:
    ///   caller   = the user URA
    ///   callee   = the device (route's callee_ura)
    ///   subject  = inner subject
    ///   ability  = `self.ability` — the PUBLIC ability name the browser
    ///              signed (NOT the daemon-local dispatch key); a mismatch
    ///              changes the canonical bytes and fails verification
    ///   args     = payload
    ///   nonce    = the user's nonce
    ///
    /// `key_id_hint` carries the base64 signer pubkey so the device's
    /// KeyResolver verifies against the presented user key (the row
    /// `register_device_pubkey` wrote into `realm-trust.toml`).
    pub fn into_wire_dispatch(
        self,
        callee_ura: &str,
        subject_ura: &str,
        payload: Vec<u8>,
    ) -> WireDispatch {
        let caller = AgentIdentity::new(self.caller_ura, UraProfile::EasynetStrictV2);
        let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
        let subject = SubjectIdentity::new(subject_ura.to_string(), UraProfile::EasynetStrictV2);
        let envelope = InvocationEnvelope::from_wire_parts(
            caller,
            callee,
            subject,
            self.nonce,
            CausalContext::None,
            self.ability,
            &payload,
        );
        let signature = CallerSignature {
            algorithm: ED25519_ALGORITHM.to_string(),
            signature: self.signature,
            key_id_hint: base64::engine::general_purpose::STANDARD.encode(self.signer_pubkey),
        };
        WireDispatch {
            envelope,
            signature,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn absent_key_returns_none() {
        let md = HashMap::new();
        assert!(OriginCaller::from_metadata(&md).unwrap().is_none());
    }

    fn sample_claim(ability: &str) -> OriginCallerClaim {
        OriginCallerClaim {
            caller_ura: "easynet:///r/localhost/user/dev".into(),
            ability: ability.into(),
            signature_b64: b64(&[9u8; 64]),
            signer_pubkey_b64: b64(&[8u8; 32]),
            nonce_b64: b64(&[7u8; 16]),
        }
    }

    #[test]
    fn resolve_prefers_typed_claim_over_legacy_metadata() {
        // Dual-write window: when BOTH carriers are present, the typed
        // field wins — it is the canonical carrier; the metadata item
        // exists only for pre-field receivers.
        let mut md = HashMap::new();
        md.insert(
            ORIGIN_CALLER_METADATA_KEY.to_string(),
            serde_json::to_string(&sample_claim("legacy.name")).unwrap(),
        );
        let claim = sample_claim("typed.name");
        let oc = OriginCaller::resolve(Some(&claim), &md).unwrap().unwrap();
        assert_eq!(oc.ability, "typed.name");

        // Field absent → metadata fallback still works.
        let oc = OriginCaller::resolve(None, &md).unwrap().unwrap();
        assert_eq!(oc.ability, "legacy.name");

        // Neither present → None.
        assert!(OriginCaller::resolve(None, &HashMap::new())
            .unwrap()
            .is_none());
    }

    #[test]
    fn claim_field_round_trips_and_old_json_decodes_without_it() {
        // Wire compat: a claim survives JSON round-trip, and frames
        // serialized by pre-field senders (no `origin_caller` key)
        // decode with the field defaulting to None.
        let claim = sample_claim("remote_desktop.create_session");
        let json = serde_json::to_string(&claim).unwrap();
        let back: OriginCallerClaim = serde_json::from_str(&json).unwrap();
        assert_eq!(claim, back);

        #[derive(serde::Deserialize)]
        struct Holder {
            #[serde(default)]
            origin_caller: Option<OriginCallerClaim>,
        }
        let holder: Holder = serde_json::from_str("{}").unwrap();
        assert!(holder.origin_caller.is_none());
    }

    #[test]
    fn valid_item_decodes() {
        let mut md = HashMap::new();
        md.insert(
            ORIGIN_CALLER_METADATA_KEY.to_string(),
            serde_json::json!({
                "caller_ura": "easynet:///r/localhost/user/dev",
                "ability": "remote_desktop.create_session",
                "signature_b64": b64(&[1u8; 64]),
                "signer_pubkey_b64": b64(&[2u8; 32]),
                "nonce_b64": b64(&[3u8; 16]),
            })
            .to_string(),
        );
        let oc = OriginCaller::from_metadata(&md).unwrap().unwrap();
        assert_eq!(oc.caller_ura, "easynet:///r/localhost/user/dev");
        assert_eq!(oc.ability, "remote_desktop.create_session");
        assert_eq!(oc.signature.len(), 64);
        assert_eq!(oc.signer_pubkey.len(), 32);
        assert_eq!(oc.nonce, [3u8; 16]);
    }

    #[test]
    fn present_but_malformed_is_error_not_silent_fallback() {
        let mut md = HashMap::new();
        // bad nonce length
        md.insert(
            ORIGIN_CALLER_METADATA_KEY.to_string(),
            serde_json::json!({
                "caller_ura": "easynet:///r/localhost/user/dev",
                "ability": "remote_desktop.create_session",
                "signature_b64": b64(&[1u8; 64]),
                "signer_pubkey_b64": b64(&[2u8; 32]),
                "nonce_b64": b64(&[3u8; 8]),
            })
            .to_string(),
        );
        assert!(OriginCaller::from_metadata(&md).is_err());

        // non-JSON
        let mut md2 = HashMap::new();
        md2.insert(ORIGIN_CALLER_METADATA_KEY.to_string(), "not json".to_string());
        assert!(OriginCaller::from_metadata(&md2).is_err());
    }

    #[test]
    fn wire_dispatch_carries_user_caller_and_pubkey_hint() {
        let mut md = HashMap::new();
        md.insert(
            ORIGIN_CALLER_METADATA_KEY.to_string(),
            serde_json::json!({
                "caller_ura": "easynet:///r/localhost/user/dev",
                "ability": "remote_desktop.create_session",
                "signature_b64": b64(&[7u8; 64]),
                "signer_pubkey_b64": b64(&[9u8; 32]),
                "nonce_b64": b64(&[3u8; 16]),
            })
            .to_string(),
        );
        let oc = OriginCaller::from_metadata(&md).unwrap().unwrap();
        let wire = oc.into_wire_dispatch(
            "easynet:///r/localhost/device/d1",
            "easynet:///r/localhost/resource/device.d1/streams/display.x",
            b"{}".to_vec(),
        );
        assert_eq!(wire.envelope.caller.ura, "easynet:///r/localhost/user/dev");
        assert_eq!(wire.envelope.ability, "remote_desktop.create_session");
        assert_eq!(wire.signature.algorithm, "ed25519");
        assert_eq!(wire.signature.key_id_hint, b64(&[9u8; 32]));
    }
}
