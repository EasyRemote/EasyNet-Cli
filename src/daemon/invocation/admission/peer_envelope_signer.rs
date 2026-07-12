//! EasyNet Daemon — Peer Envelope Signer
//! =======================================
//!
//! File: src/daemon/invocation/admission/peer_envelope_signer.rs
//! Description: Build and sign strict descriptor-bound cross-Hub invocation
//! envelopes without exposing runtime private-key material.
//!
//! Protocol Responsibility:
//! - Preserve the complete invocation tuple and explicit descriptor binding.
//! - Bind every peer signature to the configured local Hub caller URA.
//!
//! Implementation Approach:
//! - Normalize the peer envelope, derive the descriptor-bound canonical bytes,
//!   and delegate signing to an owner-bound `CanonicalSigner` capability.
//!
//! Usage Contract:
//! - Federation callers must supply both a local realm and matching Hub signer.
//! - Missing or mismatched signing authority fails before network dispatch.
//!
//! Architectural Position:
//! - Daemon admission/dispatch seam; key custody remains in the daemon key
//!   service and canonicalization remains in the Axon descriptor bridge.

use tonic::Status;

use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest, SubjectIdentity};

use crate::daemon::axon_bridge::wire_descriptor::{
    descriptor_bound_from_wire_parts, WireCallerIdentity,
};
use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::invocation::admission::register_device_pubkey::parse_realm_from_ura;
use crate::daemon::invocation::dispatch::invocation_wire::{
    try_entity_ref, SIGNED_DESCRIPTOR_REF_METADATA_KEY,
};

pub(crate) struct PeerInvokeRequest<'a> {
    caller_envelope: Option<&'a Envelope>,
    target_ura: &'a str,
    function_name: &'a str,
    arguments: Vec<u8>,
    local_realm: Option<&'a str>,
    hub_signer: Option<&'a dyn CanonicalSigner>,
}

impl<'a> PeerInvokeRequest<'a> {
    pub(crate) fn new(
        caller_envelope: Option<&'a Envelope>,
        target_ura: &'a str,
        function_name: &'a str,
        arguments: Vec<u8>,
        local_realm: Option<&'a str>,
        hub_signer: Option<&'a dyn CanonicalSigner>,
    ) -> Self {
        Self {
            caller_envelope,
            target_ura,
            function_name,
            arguments,
            local_realm,
            hub_signer,
        }
    }

    pub(crate) async fn into_invoke_request(self) -> Result<InvokeRequest, Status> {
        let mut envelope =
            build_peer_envelope(self.caller_envelope, self.target_ura, self.local_realm)?;
        let descriptor_ref = peer_descriptor_ref_for_envelope(&envelope, self.function_name)?;
        sign_peer_request_envelope(
            &mut envelope,
            self.function_name,
            &descriptor_ref,
            &self.arguments,
            self.local_realm,
            self.hub_signer,
        )
        .await?;

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
    // Subject starts as the entity the forwarded invocation acts upon: the
    // original caller when present, otherwise the target itself. Hub and User
    // URAs are valid provenance here even though Axon's descriptor-bound
    // EntityRef deliberately does not model them. `sign_peer_request_envelope`
    // owns the one canonical normalization step: unsupported subject kinds are
    // replaced by the target ability URA before canonical bytes are built.
    // Validating EntityRef here would make that normalization unreachable.
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

pub(crate) async fn sign_peer_request_envelope(
    envelope: &mut Envelope,
    ability: &str,
    descriptor_ref: &str,
    arguments: &[u8],
    local_realm: Option<&str>,
    hub_signer: Option<&dyn CanonicalSigner>,
) -> Result<String, Status> {
    let realm = local_realm.ok_or_else(|| {
        Status::failed_precondition(
            "cross-hub canonical_invoke signing requires configured local realm",
        )
    })?;
    let descriptor_ref = descriptor_ref.trim();
    if descriptor_ref.is_empty() {
        return Err(Status::invalid_argument(
            "cross-hub canonical_invoke signing requires explicit descriptor ref",
        ));
    }

    let caller_ura = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub canonical_invoke signing: caller URA missing after rewrite")
        })?
        .to_string();
    let expected_hub_ura = crate::core::ura::hub_ura(realm);
    if caller_ura != expected_hub_ura {
        return Err(Status::failed_precondition(format!(
            "cross-hub canonical_invoke signing: caller `{caller_ura}` does not match local hub `{expected_hub_ura}`"
        )));
    }
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub canonical_invoke signing: callee URA missing after rewrite")
        })?
        .to_string();
    let subject_ura = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal(
                "cross-hub canonical_invoke signing: subject URA missing after rewrite",
            )
        })?
        .to_string();
    let descriptor_subject_ura = descriptor_subject_ura_for(&callee_ura, &subject_ura, ability)
        .map_err(|err| {
            Status::internal(format!(
                "cross-hub canonical_invoke signing: derive descriptor subject: {err}"
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
            "cross-hub canonical_invoke signing: invocation_nonce must be 16 bytes",
        ));
    }
    let descriptor_ref =
        crate::daemon::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(
            &callee_ura,
            descriptor_ref,
        )
        .map_err(|err| {
            Status::internal(format!(
                "cross-hub canonical_invoke signing: invalid explicit descriptor ref for `{ability}`: {err}"
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
            "cross-hub canonical_invoke signing: build descriptor-bound envelope: {err}"
        ))
    })?;

    let hub_signer = hub_signer.ok_or_else(|| {
        Status::failed_precondition(
            "cross-hub canonical_invoke signing requires a configured hub signer",
        )
    })?;
    if hub_signer.owner_ura() != expected_hub_ura {
        return Err(Status::failed_precondition(format!(
            "cross-hub canonical_invoke signing: signer owner `{}` does not match local hub `{expected_hub_ura}`",
            hub_signer.owner_ura()
        )));
    }
    let caller_signature =
        crate::daemon::invocation::caller_signature::sign_canonical_caller_signature(
            hub_signer,
            &descriptor_bound.envelope.canonical_bytes(),
        )
        .await
        .map_err(|err| {
            Status::internal(format!(
                "cross-hub canonical_invoke signing: canonical signer failed: {err}"
            ))
        })?;
    envelope.caller_signature = Some(caller_signature);
    Ok(descriptor_ref.to_string())
}

fn peer_descriptor_ref_for_envelope(envelope: &Envelope, ability: &str) -> Result<String, Status> {
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|ura| !ura.is_empty())
        .ok_or_else(|| {
            Status::internal("cross-hub canonical_invoke signing: callee URA missing after rewrite")
        })?;
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        callee_ura,
        ability,
        crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
    )
    .map_err(|err| {
        Status::internal(format!(
            "cross-hub canonical_invoke signing: derive peer descriptor ref for `{ability}`: {err}"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::identity::self_identity::TestCanonicalSigner;
    use ed25519_dalek::Verifier as _;
    use std::sync::Arc;

    fn test_hub_signer(realm: &str) -> Arc<dyn CanonicalSigner> {
        Arc::new(TestCanonicalSigner::new(
            crate::core::ura::hub_ura(realm),
            [3u8; 32],
        ))
    }

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

    #[tokio::test]
    async fn peer_request_normalizes_hub_and_user_provenance_before_signing() {
        let signer = test_hub_signer("local");
        let expected_subject = "easynet:///r/peer/ability/hub.federation.discover".to_string();

        for provenance_ura in [
            crate::core::ura::hub_ura("origin"),
            crate::core::ura::user_ura("origin", "alice"),
        ] {
            let caller_envelope = Envelope {
                caller: Some(AgentIdentity {
                    ura: provenance_ura.clone(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            };
            let request = PeerInvokeRequest::new(
                Some(&caller_envelope),
                "easynet:///r/peer/hub",
                "federation.discover",
                br#"{"q":"chat"}"#.to_vec(),
                Some("local"),
                Some(signer.as_ref()),
            )
            .into_invoke_request()
            .await
            .unwrap_or_else(|error| {
                panic!("{provenance_ura} must normalize before signing: {error}")
            });

            let envelope = request.envelope.expect("signed peer envelope");
            assert_eq!(
                envelope.subject.expect("normalized subject").ura,
                expected_subject
            );
            assert!(envelope.caller_signature.is_some());
            assert_eq!(
                request
                    .metadata
                    .get(SIGNED_DESCRIPTOR_REF_METADATA_KEY)
                    .map(String::as_str),
                Some("easynet:///r/peer/ability/hub.federation.discover@1.0.0")
            );
        }
    }

    #[tokio::test]
    async fn sign_peer_request_preserves_causal_context() {
        let signer = test_hub_signer("local");
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
            Some(signer.as_ref()),
        )
        .await
        .unwrap();

        let signature = env
            .caller_signature
            .as_ref()
            .expect("canonical signer attaches caller signature");
        let signature = ed25519_dalek::Signature::from_slice(&signature.signature)
            .expect("ed25519 signature bytes");
        let descriptor_bound = descriptor_bound_from_wire_parts(
            env.clone(),
            "easynet:///r/peer/ability/hub.federation.discover@1.0.0".to_string(),
            br#"{"q":"chat"}"#,
            WireCallerIdentity::FromEnvelope,
        )
        .expect("descriptor-bound envelope");
        signer
            .signing_public_key()
            .expect("test signer public key")
            .verify(&descriptor_bound.envelope.canonical_bytes(), &signature)
            .expect("signature covers descriptor-bound canonical bytes");
        assert!(matches!(
            env.causal_context.and_then(|ctx| ctx.form),
            Some(easynet_axon::pb::axon::v1::causal_context::Form::Scalar(_))
        ));
    }

    #[tokio::test]
    async fn sign_peer_request_fails_closed_without_hub_signer() {
        let mut env = build_peer_envelope(
            Some(&Envelope {
                caller: Some(AgentIdentity {
                    ura: "easynet:///r/local/device/dev-a".to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            "easynet:///r/peer/device/dev-b",
            Some("local"),
        )
        .expect("peer envelope");
        let error = sign_peer_request_envelope(
            &mut env,
            "federation.discover",
            "easynet:///r/peer/ability/hub.federation.discover@1.0.0",
            br#"{"q":"chat"}"#,
            Some("local"),
            None,
        )
        .await
        .expect_err("cross-hub signing must require injected capability");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        assert!(error.message().contains("configured hub signer"));
    }

    #[tokio::test]
    async fn sign_peer_request_rejects_signer_for_another_owner() {
        let signer = test_hub_signer("other");
        let mut env = build_peer_envelope(
            Some(&Envelope {
                caller: Some(AgentIdentity {
                    ura: "easynet:///r/local/device/dev-a".to_string(),
                    ..AgentIdentity::default()
                }),
                ..Envelope::default()
            }),
            "easynet:///r/peer/device/dev-b",
            Some("local"),
        )
        .expect("peer envelope");
        let error = sign_peer_request_envelope(
            &mut env,
            "federation.discover",
            "easynet:///r/peer/ability/hub.federation.discover@1.0.0",
            br#"{"q":"chat"}"#,
            Some("local"),
            Some(signer.as_ref()),
        )
        .await
        .expect_err("signer capability must be bound to local hub owner");
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        assert!(error.message().contains("signer owner"));
    }
}
