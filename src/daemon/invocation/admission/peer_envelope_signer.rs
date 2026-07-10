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
use crate::daemon::invocation::admission::register_device_pubkey::parse_realm_from_ura;
use crate::daemon::invocation::bidi::session_initiator::SessionSigningSeed;
use crate::daemon::invocation::dispatch::invocation_wire::{
    try_entity_ref, SIGNED_DESCRIPTOR_REF_METADATA_KEY,
};

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
        .map(|realm| crate::core::ura::hub_ura(&realm))
        .ok_or_else(|| {
            Status::invalid_argument(format!("target_ura is not a valid URA: {target_ura}"))
        })?;

    let caller_ura = local_realm
        .map(crate::core::ura::hub_ura)
        .or_else(|| {
            forwarded
                .caller
                .as_ref()
                .map(|caller| caller.ura.trim().to_string())
                .filter(|ura| !ura.is_empty())
        })
        .ok_or_else(|| Status::invalid_argument("peer envelope missing caller URA"))?;
    crate::core::ura::parse_ura(&caller_ura).map_err(|err| {
        Status::invalid_argument(format!("peer envelope caller URA is invalid: {err}"))
    })?;
    crate::core::ura::parse_ura(&peer_hub_ura).map_err(|err| {
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
    crate::core::ura::parse_ura(&subject_ura).map_err(|err| {
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

    // Hub signing material is the SDK keyring entry for HubURA(realm).
    // Backend, daemon admission, and cross-hub forwarding therefore project
    // the same runtime owner identity instead of reading product-local
    // identity files.
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
    crate::core::ura::owner_ability_ura(callee_ura, ability).ok_or_else(|| {
        anyhow::anyhow!(
            "subject `{subject_ura}` is not descriptor-bound and callee `{callee_ura}` \
             cannot own ability `{ability}`"
        )
    })
}

/// Decode the base64-encoded inner envelope carried by
/// `federation.forward_invoke`. Errors map to
/// `Status::invalid_argument` with a useful message.
/// Load the Hub URA Ed25519 signing seed for `realm` from the canonical
/// daemon keyring vault. Production has no fallback: a missing keyring entry
/// means the runtime owner identity has not been provisioned correctly.
pub(crate) fn read_hub_identity_seed(realm: &str) -> Result<[u8; 32], String> {
    let hub_ura = crate::core::ura::hub_ura(realm);
    match crate::daemon::keyring::export_seed_from_default_vault(&hub_ura) {
        Ok(seed) => Ok(seed),
        Err(err) => {
            #[cfg(test)]
            {
                let _ = err;
                let hub_subject_id = easynet_axon::invocation::private_hub_subject_id(realm);
                let (seed, _pk_b64) = crate::daemon::federation::publish::derive_subject_keypair(
                    realm,
                    &hub_subject_id,
                );
                Ok(seed)
            }
            #[cfg(not(test))]
            {
                Err(format!("read Hub URA {hub_ura} from SDK keyring: {err}"))
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
        assert_eq!(caller.ura, crate::core::ura::hub_ura("local"));
        assert_eq!(callee.ura, crate::core::ura::hub_ura("peer"));
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
                ura: crate::core::ura::hub_ura("local"),
                profile: crate::daemon::invocation::DEFAULT_URA_PROFILE.to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: crate::core::ura::hub_ura("peer"),
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
                        receipt_ura: "easynet:///r/local/resource/agent.peer.signer/invocation/parent/receipt"
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
