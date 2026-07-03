// EasyNet Daemon - Hosted-Agent Delegation Minting
// =================================================
//
// File: src/daemon/invocation/hosted_agent_delegation.rs
// Description: Converts trusted loopback delegation requests into signed
//              handler metadata bound to one Axon invocation envelope.
//
// Protocol Responsibility
// -----------------------
// This module does not define Axon admission semantics. Axon still verifies the
// invocation envelope. This module owns the EasyNet daemon-local authority
// transition from "local CLI requested hosted-agent authority" to "daemon
// signed hosted-agent authority".
//
// Implementation Approach
// -----------------------
// The transport dispatcher calls this after trusted loopback admission and
// before building the Axon LocalRuntime wire dispatch. The unsigned request key
// is removed; handlers receive only the signed delegation key.
//
// Usage Contract
// --------------
// Public ingress must reject both hosted-agent metadata keys before this module
// can run. This module also rejects non-loopback callers defensively so a
// future dispatcher cannot mint authority by accident.

use std::collections::HashMap;

use easynet_axon::pb::axon::v1::Envelope;
use ed25519_dalek::Signer as _;
use tonic::Status;

use crate::daemon::ability::{
    HostedAgentDelegationEnvelopeBinding, HostedAgentDelegationRequest,
    HOSTED_AGENT_DELEGATION_METADATA_KEY, HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY,
};

/// Daemon-local issuer for hosted-agent delegation metadata.
///
/// What this is: the one transport object allowed to turn unsigned local
/// request metadata into signed hosted-agent delegation claims.
///
/// What this is not: an admission bypass. Callers must pass `loopback_admitted`
/// from `AdmissionFacade`; public requests are rejected before signing.
///
/// Invariant 1: unsigned request metadata is never forwarded to handlers.
/// Invariant 2: a signed token is minted only when the envelope caller is the
/// process-local `_system.local` identity.
pub(crate) struct HostedAgentDelegationIssuer;

impl HostedAgentDelegationIssuer {
    pub(crate) fn materialize_request_metadata(
        metadata: &HashMap<String, String>,
        envelope: &Envelope,
        loopback_admitted: bool,
        route_ability: &str,
    ) -> Result<HashMap<String, String>, Status> {
        let Some(raw_request) = metadata
            .get(HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(metadata.clone());
        };

        if !loopback_admitted {
            return Err(Status::permission_denied(
                "HOSTED_AGENT_DELEGATION_LOCAL_ONLY: unsigned hosted-agent delegation requests \
                 are accepted only on trusted loopback ingress",
            ));
        }
        if metadata
            .get(HOSTED_AGENT_DELEGATION_METADATA_KEY)
            .map(String::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(Status::invalid_argument(
                "hosted-agent delegation request cannot be supplied with a pre-signed token",
            ));
        }

        let request = HostedAgentDelegationRequest::from_metadata_value(raw_request)
            .map_err(|err| Status::invalid_argument(format!("hosted_agent_delegation: {err}")))?;
        let caller_ura = envelope
            .caller
            .as_ref()
            .map(|caller| caller.ura.trim())
            .filter(|caller| !caller.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "hosted-agent delegation request requires envelope.caller.ura",
                )
            })?;
        if caller_ura != crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
            return Err(Status::permission_denied(format!(
                "HOSTED_AGENT_DELEGATION_LOCAL_ONLY: hosted-agent delegation requests must use \
                 the local system caller, got `{caller_ura}`"
            )));
        }
        let subject_ura = envelope
            .subject
            .as_ref()
            .map(|subject| subject.ura.trim())
            .filter(|subject| !subject.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "hosted-agent delegation request requires envelope.subject.ura",
                )
            })?;
        let callee_ura = envelope
            .callee
            .as_ref()
            .map(|callee| callee.ura.trim())
            .filter(|callee| !callee.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "hosted-agent delegation request requires envelope.callee.ura",
                )
            })?;
        let binding = HostedAgentDelegationEnvelopeBinding::new(
            caller_ura,
            callee_ura,
            subject_ura,
            hex::encode(envelope.invocation_nonce.as_slice()),
            route_ability,
        )
        .map_err(|err| Status::invalid_argument(format!("hosted_agent_delegation: {err}")))?;
        let claims = request
            .into_claims("host_device", binding)
            .map_err(|err| Status::invalid_argument(format!("hosted_agent_delegation: {err}")))?;
        let signature = crate::daemon::identity::local_invocation::process_local_system_identity()
            .signing_key()
            .sign(&claims.signing_payload_bytes(caller_ura));
        let signed_metadata = claims
            .signed_metadata_value(caller_ura, &signature)
            .map_err(|err| Status::internal(format!("hosted_agent_delegation: {err}")))?;

        let mut materialized = metadata.clone();
        materialized.remove(HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY);
        materialized.insert(
            HOSTED_AGENT_DELEGATION_METADATA_KEY.to_string(),
            signed_metadata,
        );
        Ok(materialized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::HostedAgentDelegationContext;
    use crate::daemon::invocation::ProtoEnvelope;

    fn loopback_envelope() -> Envelope {
        let mut envelope = ProtoEnvelope::targeted(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            "easynet:///r/default/device/local",
            "easynet:///r/default/device/local",
        )
        .unwrap()
        .into_inner();
        envelope.invocation_nonce = vec![0x44; 16];
        envelope
    }

    #[test]
    fn materialize_request_metadata_emits_signed_token_only() {
        let request = HostedAgentDelegationRequest::new(crate::core::ura::agent_ura(
            "default", "u", "learner",
        ))
        .unwrap();
        let mut metadata = HashMap::new();
        metadata.insert(
            HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY.to_string(),
            request.metadata_value().unwrap(),
        );
        let envelope = loopback_envelope();

        let materialized = HostedAgentDelegationIssuer::materialize_request_metadata(
            &metadata,
            &envelope,
            true,
            "meta.acquire",
        )
        .unwrap();

        assert!(!materialized.contains_key(HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY));
        let raw = materialized
            .get(HOSTED_AGENT_DELEGATION_METADATA_KEY)
            .expect("signed delegation metadata");
        let binding = HostedAgentDelegationEnvelopeBinding::new(
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            "easynet:///r/default/device/local",
            "easynet:///r/default/device/local",
            hex::encode([0x44; 16]),
            "meta.acquire",
        )
        .unwrap();
        HostedAgentDelegationContext::from_signed_metadata(
            raw,
            &binding,
            crate::daemon::identity::local_invocation::system_verifying_key(),
        )
        .expect("daemon-issued token verifies with daemon local-system key");
    }

    #[test]
    fn materialize_rejects_request_outside_loopback() {
        let request = HostedAgentDelegationRequest::new(crate::core::ura::agent_ura(
            "default", "u", "learner",
        ))
        .unwrap();
        let metadata = HashMap::from([(
            HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY.to_string(),
            request.metadata_value().unwrap(),
        )]);
        let envelope = loopback_envelope();

        let err = HostedAgentDelegationIssuer::materialize_request_metadata(
            &metadata,
            &envelope,
            false,
            "meta.acquire",
        )
        .unwrap_err();

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }
}
