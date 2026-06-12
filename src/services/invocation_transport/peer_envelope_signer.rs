// EasyNet Daemon — Peer Envelope Signer
// =======================================
//
// File: src/services/invocation_transport/peer_envelope_signer.rs
// Description: Cross-hub peer request construction (commit-plan-2
//              Axis E / E4 module): the strict hub-to-hub envelope a
//              federation dialer attaches to a rebuilt peer
//              `InvokeRequest`, the hub-identity signature over it,
//              and the inner-envelope base64 decode used on receive.
//              Consumed by the unary dispatcher's forward_invoke and
//              backend-proxy arms.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use tonic::Status;

use easynet_axon::pb::axon::v1::{AgentIdentity, CallerSignature, Envelope, SubjectIdentity};

use crate::services::invocation_transport::register_device_pubkey::parse_realm_from_ura;
use crate::services::invocation_transport::session_initiator::SessionSigningSeed;

/// Build the strict envelope the cross-hub dialer attaches to the
/// rebuilt peer `InvokeRequest`.
///
/// This is a new hub-to-hub invocation, not a verbatim re-send:
/// `caller = local hub`, `callee = target hub`, and `subject =
/// original caller` when present. Every URA must parse through the
/// canonical URA parser before the peer request is sent.
pub(crate) fn build_peer_envelope(
    caller_envelope: Option<&Envelope>,
    target_ura: &str,
    local_realm: Option<&str>,
) -> Result<Envelope, Status> {
    use rand::RngCore as _;

    let mut forwarded = caller_envelope.cloned().unwrap_or_default();
    let peer_hub_ura = parse_realm_from_ura(target_ura)
        .map(|realm| crate::ura::hub_ura(&realm))
        .ok_or_else(|| {
            Status::invalid_argument(format!("target_ura is not a valid URA: {target_ura}"))
        })?;

    let caller_ura = local_realm
        .map(crate::ura::hub_ura)
        .or_else(|| {
            forwarded
                .caller
                .as_ref()
                .map(|caller| caller.ura.trim().to_string())
                .filter(|ura| !ura.is_empty())
        })
        .ok_or_else(|| Status::invalid_argument("peer envelope missing caller URA"))?;
    crate::ura::parse_ura(&caller_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope caller URA is invalid: {err}"))
    })?;
    crate::ura::parse_ura(&peer_hub_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope callee URA is invalid: {err}"))
    })?;
    let subject_ura = caller_envelope
        .and_then(|env| env.caller.as_ref())
        .map(|caller| caller.ura.trim().to_string())
        .filter(|ura| !ura.is_empty())
        .unwrap_or_else(|| peer_hub_ura.clone());
    crate::ura::parse_ura(&subject_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope subject URA is invalid: {err}"))
    })?;

    let profile = crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string();
    forwarded.caller = Some(AgentIdentity {
        ura: caller_ura,
        profile: profile.clone(),
    });
    forwarded.callee = Some(AgentIdentity {
        ura: peer_hub_ura,
        profile: profile.clone(),
    });
    forwarded.subject = Some(SubjectIdentity {
        ura: subject_ura,
        profile,
    });

    if forwarded.invocation_nonce.len() != 16 {
        let mut nonce = vec![0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        forwarded.invocation_nonce = nonce;
    }

    Ok(forwarded)
}

pub(crate) fn sign_peer_request_envelope(
    envelope: &mut Envelope,
    ability: &str,
    arguments: &[u8],
    local_realm: Option<&str>,
    hub_signing_seed: Option<&SessionSigningSeed>,
) -> Result<(), Status> {
    let Some(realm) = local_realm else {
        return Ok(());
    };

    use easynet_axon::invocation::axiom::{
        canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
        InvocationEnvelope, SubjectIdentity as AxiomSubjectIdentity, UraProfile,
    };
    use ed25519_dalek::{Signer as _, SigningKey};
    use sha2::{Digest, Sha256};

    envelope.causal_context = None;

    let caller_ura = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: caller URA missing after rewrite")
        })?;
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: callee URA missing after rewrite")
        })?;
    let subject_ura = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: subject URA missing after rewrite")
        })?;
    let invocation_nonce: [u8; 16] =
        envelope
            .invocation_nonce
            .as_slice()
            .try_into()
            .map_err(|_| {
                Status::internal(
                    "cross-hub forward_invoke signing: invocation_nonce must be 16 bytes",
                )
            })?;

    let mut hasher = Sha256::new();
    hasher.update(arguments);
    let args_digest: [u8; 32] = hasher.finalize().into();

    // Hub identity is a fresh-random Ed25519 seed minted by
    // backend's `LoadOrInitHubIdentity` at first boot and persisted
    // to `${HOME}/.easynet-hub/<realm>/identity.json` (see backend
    // `runtime/subject_context.go::backendIdentityRecord`). The
    // pre-fix path used `derive_subject_keypair(realm,
    // "easynet:prv:hub:{realm}")` — deterministically derived from
    // SHA256(realm + subject_id) — which produced a DIFFERENT key
    // than the trust-anchor entry (sourced from `identity.json`).
    // Peer hubs verifying via `federation.resolve_key` saw a
    // signature/key mismatch and rejected with
    // `CALLER_SIGNATURE_INVALID:caller_signature_invalid`.
    //
    // Read the on-disk seed in production so the signing key
    // matches the pubkey the trust anchor advertises. Tests stage
    // an identity.json under their per-test HomeGuard root via
    // `stage_test_hub_identity` so the same code path covers both.
    let hub_seed = match hub_signing_seed.copied() {
        Some(seed) => seed,
        None => read_hub_identity_seed(realm).map_err(|err| {
            Status::internal(format!(
                "cross-hub forward_invoke signing: load hub identity seed for realm `{realm}`: {err}"
            ))
        })?,
    };
    let signing_key = SigningKey::from_bytes(&hub_seed);
    let axiom_envelope = InvocationEnvelope {
        caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
        callee: AxiomAgentIdentity::new(callee_ura, UraProfile::EasynetStrictV2),
        subject: AxiomSubjectIdentity::new(subject_ura, UraProfile::EasynetStrictV2),
        ability: ability.to_string(),
        args_digest,
        invocation_nonce,
        causal_context: CausalContext::None,
    };
    let signature = signing_key.sign(&canonical_invocation_bytes(&axiom_envelope));
    envelope.caller_signature = Some(CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature.to_bytes().to_vec(),
        ..CallerSignature::default()
    });
    Ok(())
}

/// Decode the base64-encoded inner envelope carried by
/// `federation.forward_invoke`. Errors map to
/// `Status::invalid_argument` with a useful message.
/// Tests fall back to the deterministic
/// `derive_subject_keypair(realm, "easynet:prv:hub:{realm}")`
/// seed when the on-disk file is missing — this preserves the
/// pre-fix wire shape for in-process unit tests that don't stage
/// a `~/.easynet-hub/<realm>/identity.json` fixture, while
/// production daemons (which always have the file, written by
/// backend's first-boot bootstrap) take the real-seed path.
/// The fallback is `cfg(test)`-gated so an accidentally-missing
/// identity file in production fails loudly rather than silently
/// substituting a key the peer hub will reject.
/// Load the hub's Ed25519 signing seed for `realm` from the
/// on-disk identity file backend's `LoadOrInitHubIdentity` writes
/// at first boot. File shape mirrors backend
/// `runtime/subject_context.go::backendIdentityRecord`:
///
/// ```json
/// {
///   "private_key_seed_hex": "<64-hex>",
///   "agent_ura": "easynet:///r/<realm>/hub",
///   "created_at_unix_ms": <int>
/// }
/// ```
///
/// Path: `${HOME}/.easynet-hub/<realm>/identity.json`. In
/// production hub containers `HOME=/srv/easynet`, so the resolved
/// path is `/srv/easynet/.easynet-hub/<realm>/identity.json`.
///
/// Returns the 32-byte seed. Errors propagate as `String` and the
/// caller wraps them in `Status::internal`. This helper is only
/// used by the cross-hub `federation.forward_invoke` signing path
/// today; the seed is the same one the trust anchor's hub entry
/// advertises as `public_key_b64`, so a peer's
/// `federation.resolve_key` lookup → signature verify round trip
/// closes cleanly.
pub(crate) fn read_hub_identity_seed(realm: &str) -> Result<[u8; 32], String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME unset".to_string())?;
    let path = std::path::Path::new(&home)
        .join(".easynet-hub")
        .join(realm)
        .join("identity.json");
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            #[derive(serde::Deserialize)]
            struct HubIdentityRecord {
                private_key_seed_hex: String,
            }
            let parsed: HubIdentityRecord = serde_json::from_str(&raw)
                .map_err(|err| format!("parse {}: {err}", path.display()))?;
            let seed_bytes = hex::decode(parsed.private_key_seed_hex.trim())
                .map_err(|err| format!("decode hex from {}: {err}", path.display()))?;
            if seed_bytes.len() != 32 {
                return Err(format!(
                    "{} private_key_seed_hex must decode to 32 bytes, got {}",
                    path.display(),
                    seed_bytes.len()
                ));
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&seed_bytes);
            Ok(seed)
        }
        Err(err) => {
            #[cfg(test)]
            {
                let _ = err;
                // Test fallback: deterministic derive matches the
                // pre-fix wire shape so existing unit tests that
                // don't stage an `identity.json` fixture stay
                // green. Production never takes this path (see
                // function-level docs).
                let hub_subject_id = easynet_axon::invocation::private_hub_subject_id(realm);
                let (seed, _pk_b64) =
                    crate::runtime::publish::derive_subject_keypair(realm, &hub_subject_id);
                Ok(seed)
            }
            #[cfg(not(test))]
            {
                Err(format!("read {}: {err}", path.display()))
            }
        }
    }
}

pub(crate) fn decode_inner_envelope(b64: &str) -> Result<Vec<u8>, Status> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    if b64.is_empty() {
        return Ok(Vec::new());
    }
    STANDARD.decode(b64).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.forward_invoke: inner_envelope_b64 is not valid base64: {err}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_peer_envelope_maps_to_hub_tuple_with_profile() {
        let caller_envelope = Envelope {
            caller: Some(AgentIdentity {
                ura: "easynet:///r/local/device/dev-a".to_string(),
                ..AgentIdentity::default()
            }),
            ..Envelope::default()
        };
        let env = build_peer_envelope(
            Some(&caller_envelope),
            "easynet:///r/peer/device/dev-b",
            Some("local"),
        )
        .unwrap();

        let caller = env.caller.unwrap();
        let callee = env.callee.unwrap();
        let subject = env.subject.unwrap();
        assert_eq!(caller.ura, crate::ura::hub_ura("local"));
        assert_eq!(callee.ura, crate::ura::hub_ura("peer"));
        assert_eq!(subject.ura, "easynet:///r/local/device/dev-a");
        assert_eq!(
            caller.profile,
            crate::services::invocation_transport::DEFAULT_URA_PROFILE
        );
        assert_eq!(
            callee.profile,
            crate::services::invocation_transport::DEFAULT_URA_PROFILE
        );
        assert_eq!(
            subject.profile,
            crate::services::invocation_transport::DEFAULT_URA_PROFILE
        );
        assert_eq!(env.invocation_nonce.len(), 16);
    }

    #[test]
    fn build_peer_envelope_rejects_bad_target_ura() {
        let err = build_peer_envelope(None, "agent://dev-b", Some("local")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
}
