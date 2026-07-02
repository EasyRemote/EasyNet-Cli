// EasyNet Daemon — Peer Envelope Signer
// =======================================
//
// File: src/daemon/invocation/peer_envelope_signer.rs
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

use easynet_axon::pb::axon::v1::{
    AgentIdentity, CallerSignature, Envelope, InvokeRequest, SubjectIdentity,
};

use crate::daemon::axon_bridge::wire_descriptor::{
    descriptor_bound_from_wire_parts, WireCallerIdentity,
};
use crate::daemon::invocation::invocation_wire::{
    try_entity_ref, SIGNED_DESCRIPTOR_REF_METADATA_KEY,
};
use crate::daemon::invocation::register_device_pubkey::parse_realm_from_ura;
use crate::daemon::invocation::session_initiator::SessionSigningSeed;

pub(crate) struct PeerInvokeRequest<'a> {
    caller_envelope: Option<&'a Envelope>,
    target_ura: &'a str,
    function_name: &'a str,
    arguments: Vec<u8>,
    local_realm: Option<&'a str>,
    hub_signing_seed: Option<&'a SessionSigningSeed>,
}

impl<'a> PeerInvokeRequest<'a> {
    pub(crate) fn new(
        caller_envelope: Option<&'a Envelope>,
        target_ura: &'a str,
        function_name: &'a str,
        arguments: Vec<u8>,
        local_realm: Option<&'a str>,
        hub_signing_seed: Option<&'a SessionSigningSeed>,
    ) -> Self {
        Self {
            caller_envelope,
            target_ura,
            function_name,
            arguments,
            local_realm,
            hub_signing_seed,
        }
    }

    pub(crate) fn into_invoke_request(self) -> Result<InvokeRequest, Status> {
        let mut envelope =
            build_peer_envelope(self.caller_envelope, self.target_ura, self.local_realm)?;
        let descriptor_ref = peer_descriptor_ref_for_envelope(&envelope, self.function_name)?;
        sign_peer_request_envelope(
            &mut envelope,
            self.function_name,
            &descriptor_ref,
            &self.arguments,
            self.local_realm,
            self.hub_signing_seed,
        )?;

        let mut request = InvokeRequest {
            envelope: Some(envelope),
            function_name: self.function_name.to_string(),
            arguments: self.arguments,
            ..InvokeRequest::default()
        };
        request.metadata.insert(
            SIGNED_DESCRIPTOR_REF_METADATA_KEY.to_string(),
            descriptor_ref,
        );
        Ok(request)
    }
}

/// Build the strict envelope the cross-hub dialer attaches to the
/// rebuilt peer `InvokeRequest`.
///
/// This is a new hub-to-hub invocation, not a verbatim re-send:
/// `caller = local hub`, `callee = target hub`, and `subject =
/// original caller` when present. Signing normalizes the subject to a
/// descriptor-bound EntityRef when the original caller is a Hub/User URA.
/// Every URA must parse through the canonical URA parser before the peer
/// request is sent.
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
    // Subject is the entity the forwarded invocation acts upon: the
    // original caller when present, otherwise the target itself. It must
    // be a device/agent/ability/resource — never the peer hub, which the
    // descriptor-bound subject contract rejects (subject_ref_kind
    // unsupported:Hub). The hub is the transport callee, not the subject.
    let subject_ura = caller_envelope
        .and_then(|env| env.caller.as_ref())
        .map(|caller| caller.ura.trim().to_string())
        .filter(|ura| !ura.is_empty())
        .unwrap_or_else(|| target_ura.trim().to_string());
    crate::ura::parse_ura(&subject_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope subject URA is invalid: {err}"))
    })?;

    let profile = crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string();
    forwarded.caller = Some(AgentIdentity {
        ura: caller_ura,
        profile: profile.clone(),
    });
    forwarded.callee = Some(AgentIdentity {
        ura: peer_hub_ura,
        profile: profile.clone(),
    });
    forwarded.subject = Some(SubjectIdentity {
        ura: subject_ura.clone(),
        profile,
    });
    try_entity_ref(subject_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope subject is invalid: {err}"))
    })?;

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
    descriptor_ref: &str,
    arguments: &[u8],
    local_realm: Option<&str>,
    hub_signing_seed: Option<&SessionSigningSeed>,
) -> Result<String, Status> {
    let realm = local_realm.ok_or_else(|| {
        Status::failed_precondition(
            "cross-hub forward_invoke signing requires configured local realm",
        )
    })?;
    let descriptor_ref = descriptor_ref.trim();
    if descriptor_ref.is_empty() {
        return Err(Status::invalid_argument(
            "cross-hub forward_invoke signing requires explicit descriptor ref",
        ));
    }

    use ed25519_dalek::{Signer as _, SigningKey};

    envelope
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
        })?
        .to_string();
    let subject_ura = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: subject URA missing after rewrite")
        })?
        .to_string();
    let descriptor_subject_ura = descriptor_subject_ura_for(&callee_ura, &subject_ura, ability)
        .map_err(|err| {
            Status::internal(format!(
                "cross-hub forward_invoke signing: derive descriptor subject: {err}"
            ))
        })?;
    if descriptor_subject_ura != subject_ura {
        let profile = envelope
            .subject
            .as_ref()
            .map(|subject| subject.profile.clone())
            .filter(|profile| !profile.trim().is_empty())
            .unwrap_or_else(|| crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string());
        envelope.subject = Some(SubjectIdentity {
            ura: descriptor_subject_ura.clone(),
            profile,
        });
    }
    if envelope.invocation_nonce.len() != 16 {
        return Err(Status::internal(
            "cross-hub forward_invoke signing: invocation_nonce must be 16 bytes",
        ));
    }
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(
            &callee_ura,
            descriptor_ref,
        )
        .map_err(|err| {
            Status::internal(format!(
                "cross-hub forward_invoke signing: invalid explicit descriptor ref for `{ability}`: {err}"
            ))
        })?;

    let descriptor_bound = descriptor_bound_from_wire_parts(
        envelope.clone(),
        descriptor_ref.clone(),
        arguments,
        WireCallerIdentity::FromEnvelope,
    )
    .map_err(|err| {
        Status::internal(format!(
            "cross-hub forward_invoke signing: build descriptor-bound envelope: {err}"
        ))
    })?;

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
    let signature = signing_key.sign(&descriptor_bound.envelope.canonical_bytes());
    envelope.caller_signature = Some(CallerSignature {
        algorithm: "ed25519".to_string(),
        signature: signature.to_bytes().to_vec(),
        ..CallerSignature::default()
    });
    Ok(descriptor_ref.to_string())
}

fn peer_descriptor_ref_for_envelope(envelope: &Envelope, ability: &str) -> Result<String, Status> {
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub forward_invoke signing: callee URA missing after rewrite")
        })?;
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        callee_ura,
        ability,
        crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
    )
    .map_err(|err| {
        Status::internal(format!(
            "cross-hub forward_invoke signing: derive peer descriptor ref for `{ability}`: {err}"
        ))
    })
}

fn descriptor_subject_ura_for(
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
) -> anyhow::Result<String> {
    if try_entity_ref(subject_ura.to_string()).is_ok() {
        return Ok(subject_ura.to_string());
    }
    crate::ura::owner_ability_ura(callee_ura, ability).ok_or_else(|| {
        anyhow::anyhow!(
            "subject `{subject_ura}` is not descriptor-bound and callee `{callee_ura}` \
             cannot own ability `{ability}`"
        )
    })
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
                let (seed, _pk_b64) = crate::daemon::federation::publish::derive_subject_keypair(
                    realm,
                    &hub_subject_id,
                );
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
            crate::daemon::invocation::DEFAULT_URA_PROFILE
        );
        assert_eq!(
            callee.profile,
            crate::daemon::invocation::DEFAULT_URA_PROFILE
        );
        assert_eq!(
            subject.profile,
            crate::daemon::invocation::DEFAULT_URA_PROFILE
        );
        assert_eq!(env.invocation_nonce.len(), 16);
    }

    #[test]
    fn build_peer_envelope_rejects_bad_target_ura() {
        let err = build_peer_envelope(None, "agent://dev-b", Some("local")).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn sign_peer_request_preserves_causal_context() {
        let mut env = Envelope {
            caller: Some(AgentIdentity {
                ura: crate::ura::hub_ura("local"),
                profile: crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: crate::ura::hub_ura("peer"),
                profile: crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string(),
            }),
            subject: Some(SubjectIdentity {
                ura: "easynet:///r/local/device/dev-a".to_string(),
                profile: crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string(),
            }),
            invocation_nonce: vec![9u8; 16],
            causal_context: Some(easynet_axon::pb::axon::v1::CausalContext {
                form: Some(easynet_axon::pb::axon::v1::causal_context::Form::Scalar(
                    easynet_axon::pb::axon::v1::ReceiptRef {
                        receipt_hash: vec![7u8; 32],
                        receipt_ura: "easynet:///r/local/resource/invocations/parent/receipt"
                            .to_string(),
                    },
                )),
            }),
            ..Envelope::default()
        };

        sign_peer_request_envelope(
            &mut env,
            // A hub ability is owner-local-dotted (`hub.<name>`); a bare
            // single-segment name has no valid hub descriptor URA. Use a
            // real federation ability, as every production caller does.
            "federation.discover",
            "easynet:///r/peer/ability/hub.federation.discover@1.0.0",
            br#"{"q":"chat"}"#,
            Some("local"),
            Some(&[3u8; 32]),
        )
        .unwrap();

        assert!(env.caller_signature.is_some());
        assert!(matches!(
            env.causal_context.and_then(|ctx| ctx.form),
            Some(easynet_axon::pb::axon::v1::causal_context::Form::Scalar(_))
        ));
    }
}
