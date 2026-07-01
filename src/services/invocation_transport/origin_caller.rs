// EasyNet CLI — invoke_remote inner user-caller pass-through
// ===========================================================
//
// File: src/services/invocation_transport/origin_caller.rs
// Description: Validate the typed origin-caller claim attached to an
//              `runtime.invoke_remote` request and rebuild the
//              descriptor-bound inner invocation envelope with the real
//              originating principal as caller.
//
// Why this exists
// ---------------
// The outer `runtime.invoke_remote` envelope is the transport request.
// The origin-caller claim is the authority material for the inner
// descriptor-bound invocation. When present, the executing daemon
// verifies the claim against Axon descriptor-bound canonical bytes and
// runs the inner ability with the real caller. A forged value fails
// Axon admission.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use base64::Engine as _;
use easynet_axon::invocation::{
    AgentIdentity, CallerSignature, CausalContext, DescriptorBoundEnvelope,
    DescriptorBoundEnvelopeParts, EntityRef, SubjectIdentity, UraProfile,
};

use crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire;
use crate::runtime::axon_bridge::dispatch_shim::{WireDispatch, WireDispatchIngress};

const ED25519_ALGORITHM: &str = "ed25519";
const ED25519_SIGNATURE_LEN: usize = 64;
const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// The claim's wire shape is protocol material and lives in the Axon
/// SDK (it rides `ForwardInvokeRequest` as well as the CLI-owned
/// session frames). Re-exported here so every existing dispatch-site
/// path keeps reading naturally.
pub use easynet_axon::OriginCallerClaim;

/// Decoded, validated origin-caller authorization.
#[derive(Debug, Clone)]
pub struct OriginCaller {
    pub caller_ura: String,
    /// Public ability name the browser signed (canonical `ability` field).
    pub ability: String,
    pub descriptor_version: String,
    pub signature: Vec<u8>,
    pub signer_pubkey: Vec<u8>,
    pub nonce: [u8; 16],
}

impl OriginCaller {
    /// Resolve the origin-caller authority for one dispatch. Only the typed
    /// first-class `claim` field is authoritative; ordinary metadata never
    /// carries caller identity. Returns `None` when no typed claim is present
    /// (the common, non-user path). Returns `Err` when a claim IS present but
    /// malformed — a present-but-broken authority must not silently degrade to
    /// `_system`.
    pub fn resolve(claim: Option<&OriginCallerClaim>) -> anyhow::Result<Option<Self>> {
        if let Some(claim) = claim {
            return Self::from_claim(claim.clone()).map(Some);
        }
        Ok(None)
    }

    /// Validate + decode a typed claim into the dispatchable form.
    pub fn from_claim(wire: OriginCallerClaim) -> anyhow::Result<Self> {
        if wire.caller_ura.trim().is_empty() {
            anyhow::bail!("origin_caller: empty caller_ura");
        }
        crate::ura::parse_ura(wire.caller_ura.trim())
            .map_err(|e| anyhow::anyhow!("origin_caller: invalid caller_ura: {e}"))?;
        if wire.ability.trim().is_empty() {
            anyhow::bail!("origin_caller: empty ability");
        }
        if wire.descriptor_version.trim().is_empty() {
            anyhow::bail!("origin_caller: empty descriptor_version");
        }
        let b64 = base64::engine::general_purpose::STANDARD;
        let signature = b64
            .decode(wire.signature_b64.trim())
            .map_err(|e| anyhow::anyhow!("origin_caller: bad signature_b64: {e}"))?;
        if signature.len() != ED25519_SIGNATURE_LEN {
            anyhow::bail!(
                "origin_caller: signature must be {ED25519_SIGNATURE_LEN} bytes, got {}",
                signature.len()
            );
        }
        let signer_pubkey = b64
            .decode(wire.signer_pubkey_b64.trim())
            .map_err(|e| anyhow::anyhow!("origin_caller: bad signer_pubkey_b64: {e}"))?;
        if signer_pubkey.len() != ED25519_PUBLIC_KEY_LEN {
            anyhow::bail!(
                "origin_caller: signer_pubkey must be {ED25519_PUBLIC_KEY_LEN} bytes, got {}",
                signer_pubkey.len()
            );
        }
        let nonce_vec = b64
            .decode(wire.nonce_b64.trim())
            .map_err(|e| anyhow::anyhow!("origin_caller: bad nonce_b64: {e}"))?;
        let nonce: [u8; 16] = nonce_vec.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!(
                "origin_caller: nonce must be 16 bytes, got {}",
                nonce_vec.len()
            )
        })?;
        Ok(Self {
            caller_ura: wire.caller_ura,
            ability: wire.ability,
            descriptor_version: wire.descriptor_version,
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

    /// Build the inner descriptor-bound `WireDispatch` (envelope +
    /// signature + payload) for Axon external signed dispatch. The
    /// envelope MUST reproduce the exact invocation object the browser
    /// signed:
    ///   caller   = the user URA
    ///   callee   = the device (route's callee_ura)
    ///   subject  = inner subject EntityRef
    ///   ability  = `self.ability` — the PUBLIC ability name the browser
    ///              signed (NOT the daemon-local dispatch key); a mismatch
    ///              changes the canonical bytes and fails verification
    ///   descriptor_version = the governed descriptor version the signer used
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
    ) -> anyhow::Result<WireDispatch> {
        let caller = AgentIdentity::new(self.caller_ura, UraProfile::EasynetStrictV2);
        let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
        let (subject, _subject_ref) = descriptor_subject_identity(subject_ura)?;
        let ability =
            ability_descriptor_ref_for_wire(callee_ura, &self.ability, &self.descriptor_version)
                .map_err(|err| anyhow::anyhow!("origin_caller: descriptor-bound ability: {err}"))?;
        let envelope = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
            caller,
            callee,
            ability,
            subject,
            invocation_nonce: self.nonce,
            causal_context: CausalContext::None,
            args_bytes: &payload,
        })
        .map_err(|err| anyhow::anyhow!("origin_caller: descriptor-bound envelope: {err}"))?;
        let signature = CallerSignature {
            algorithm: ED25519_ALGORITHM.to_string(),
            signature: self.signature,
            key_id_hint: base64::engine::general_purpose::STANDARD.encode(self.signer_pubkey),
        };
        Ok(WireDispatch {
            envelope,
            ingress: WireDispatchIngress::ExternalSigned(signature),
            payload,
            request_metadata: Default::default(),
            trace_id: String::new(),
        })
    }
}

fn descriptor_subject_identity(subject_ura: &str) -> anyhow::Result<(SubjectIdentity, EntityRef)> {
    let subject = SubjectIdentity::new(subject_ura.to_string(), UraProfile::EasynetStrictV2);
    let subject_ref = EntityRef::try_from_subject_identity(&subject)
        .map_err(|err| anyhow::anyhow!("origin_caller: invalid descriptor subject: {err}"))?;
    Ok((subject, subject_ref))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn sample_claim(ability: &str) -> OriginCallerClaim {
        OriginCallerClaim {
            caller_ura: "easynet:///r/localhost/user/dev".into(),
            ability: ability.into(),
            descriptor_version: crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION.into(),
            signature_b64: b64(&[9u8; 64]),
            signer_pubkey_b64: b64(&[8u8; 32]),
            nonce_b64: b64(&[7u8; 16]),
        }
    }

    #[test]
    fn absent_claim_returns_none() {
        assert!(OriginCaller::resolve(None).unwrap().is_none());
    }

    #[test]
    fn typed_claim_decodes() {
        let claim = sample_claim("typed.name");
        let oc = OriginCaller::resolve(Some(&claim)).unwrap().unwrap();
        assert_eq!(oc.caller_ura, "easynet:///r/localhost/user/dev");
        assert_eq!(oc.ability, "typed.name");
        assert_eq!(
            oc.descriptor_version,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );
        assert_eq!(oc.signature.len(), 64);
        assert_eq!(oc.signer_pubkey.len(), 32);
        assert_eq!(oc.nonce, [7u8; 16]);
    }

    #[test]
    fn resolve_uses_only_typed_claim() {
        let claim = sample_claim("typed.name");
        let oc = OriginCaller::resolve(Some(&claim)).unwrap().unwrap();
        assert_eq!(oc.ability, "typed.name");
        assert_eq!(
            oc.descriptor_version,
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );
    }

    #[test]
    fn claim_field_round_trips() {
        let claim = sample_claim("remote_desktop.create_session");
        let json = serde_json::to_string(&claim).unwrap();
        let back: OriginCallerClaim = serde_json::from_str(&json).unwrap();
        assert_eq!(claim, back);
    }

    #[test]
    fn present_but_malformed_is_error() {
        let mut bad_nonce = sample_claim("remote_desktop.create_session");
        bad_nonce.nonce_b64 = b64(&[3u8; 8]);
        assert!(OriginCaller::from_claim(bad_nonce).is_err());

        let mut bad_version = sample_claim("remote_desktop.create_session");
        bad_version.descriptor_version.clear();
        assert!(OriginCaller::from_claim(bad_version).is_err());

        let mut bad_caller = sample_claim("remote_desktop.create_session");
        bad_caller.caller_ura = "not-a-ura".to_string();
        assert!(OriginCaller::from_claim(bad_caller)
            .unwrap_err()
            .to_string()
            .contains("invalid caller_ura"));

        let mut bad_signature = sample_claim("remote_desktop.create_session");
        bad_signature.signature_b64 = b64(&[1u8; 63]);
        assert!(OriginCaller::from_claim(bad_signature)
            .unwrap_err()
            .to_string()
            .contains("signature must be 64 bytes"));

        let mut bad_pubkey = sample_claim("remote_desktop.create_session");
        bad_pubkey.signer_pubkey_b64 = b64(&[2u8; 31]);
        assert!(OriginCaller::from_claim(bad_pubkey)
            .unwrap_err()
            .to_string()
            .contains("signer_pubkey must be 32 bytes"));
    }

    #[test]
    fn origin_dispatch_rejects_invalid_subject_without_fallback() {
        let claim = sample_claim("chat");
        let err = OriginCaller::from_claim(claim)
            .unwrap()
            .into_wire_dispatch(
                "easynet:///r/localhost/agent/dev.assistant",
                "not-a-subject-ura",
                b"{}".to_vec(),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("invalid descriptor subject"),
            "subject must fail at the supplied value, got {err}"
        );
    }

    #[test]
    fn wire_dispatch_carries_user_caller_and_pubkey_hint() {
        let mut claim = sample_claim("remote_desktop.create_session");
        claim.signature_b64 = b64(&[7u8; 64]);
        claim.signer_pubkey_b64 = b64(&[9u8; 32]);
        claim.nonce_b64 = b64(&[3u8; 16]);

        let oc = OriginCaller::from_claim(claim).unwrap();
        let wire = oc
            .into_wire_dispatch(
                "easynet:///r/localhost/device/d1",
                "easynet:///r/localhost/resource/device.d1/streams/display.x",
                b"{}".to_vec(),
            )
            .unwrap();
        assert_eq!(
            wire.envelope.envelope().caller.ura,
            "easynet:///r/localhost/user/dev"
        );
        let expected_ability = format!(
            "{}@{}",
            crate::ura::owner_ability_ura(
                "easynet:///r/localhost/device/d1",
                "remote_desktop.create_session"
            )
            .expect("ability URA"),
            crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION
        );
        assert_eq!(wire.envelope.envelope().ability, expected_ability);
        let WireDispatchIngress::ExternalSigned(signature) = &wire.ingress else {
            panic!("origin caller dispatch must preserve external signed ingress");
        };
        assert_eq!(signature.algorithm, "ed25519");
        assert_eq!(signature.key_id_hint, b64(&[9u8; 32]));
        assert_eq!(
            wire.envelope.envelope().subject.ura,
            "easynet:///r/localhost/resource/device.d1/streams/display.x"
        );
    }
}
