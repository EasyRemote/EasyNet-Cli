//! EasyNet CLI - LocalRuntime descriptor-bound request factory
//! ==========================================================
//!
//! File: src/daemon/axon_bridge/local_runtime_request.rs
//! Description: Typed boundary from daemon ingress classification to
//!              Axon's public descriptor-bound request constructors.
//!
//! Protocol Responsibility:
//! - Preserve Axon's ownership of descriptor-bound admission, signature
//!   verification, nonce replay, receipt proof normalization, and launch.
//! - Prevent EasyNet-Cli from constructing or mirroring Axon runtime-internal
//!   admission state.
//!
//! Implementation Approach:
//! - `LocalRuntimeIngress::ExternalSigned` requires a caller signature and
//!   maps only to `DescriptorBoundInvocationRequest::externally_signed`.
//! - `SystemInvocationIssuer` is the only entry that may bind the
//!   process-local `_system.local` signing capability.
//!
//! Usage Contract:
//! - Daemon policy gates classify ingress before this factory is called.
//! - This factory attaches trace and dispatch keys without changing
//!   admission semantics. Receipt proof facts are owned by Axon's
//!   registration-time proof binding, not by per-request CLI injection.
//!
//! Architectural Position:
//! - EasyNet-Cli daemon/Axon bridge. Axon SDK remains the protocol source of
//!   truth; CLI owns only runtime admission and request plumbing.

use std::collections::HashMap;

use axon_sdk::invocation::{
    AgentIdentity, AxonError, CallMode as AxonInvocationCallMode, CallerSignature,
    CanonicalEnvelopeBuilder, CausalContext, DescriptorBoundEnvelope,
    DescriptorBoundInvocationRequest, EntityRef, InvocationDerivationPolicy, SubjectIdentity,
    UraProfile,
};

use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
use crate::daemon::identity::local_invocation::{
    sign_system_canonical, system_agent_identity, LOCAL_SYSTEM_AGENT_URA,
};
use crate::daemon::identity::self_identity::CanonicalSigner;

/// Runtime-owned metadata projection of the operational trace id.
///
/// Arbitrary request metadata cannot assert this key: the canonical factory
/// removes caller-supplied values and writes only the dedicated trace field
/// admitted by the daemon boundary. Invocation-scoped child gateways use the
/// projection to continue traces without parsing Mission arguments.
pub(crate) const AXON_TRACE_CONTEXT_METADATA_KEY: &str = "axon.trace_id";

/// Daemon classification of one descriptor-bound ingress into Axon
/// `LocalRuntime`.
#[derive(Debug)]
pub(crate) enum LocalRuntimeIngress {
    /// External caller material that still needs Axon public admission.
    ExternalSigned {
        envelope: DescriptorBoundEnvelope,
        signature: CallerSignature,
        payload: Vec<u8>,
    },
}

/// Optional execution metadata that does not decide admission semantics.
#[derive(Debug, Default)]
pub(crate) struct LocalRuntimeRequestOptions {
    trace_id: String,
    request_metadata: HashMap<String, String>,
}

impl LocalRuntimeRequestOptions {
    #[must_use]
    pub(crate) fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = trace_id.into();
        self
    }

    #[must_use]
    #[cfg(feature = "axon-pb")]
    pub(crate) fn with_request_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.request_metadata = metadata;
        self
    }
}

/// Single constructor boundary for Axon descriptor-bound requests.
pub(crate) struct LocalRuntimeRequestFactory;

impl LocalRuntimeRequestFactory {
    pub(crate) fn request_for(
        mode: AxonInvocationCallMode,
        ingress: LocalRuntimeIngress,
        options: LocalRuntimeRequestOptions,
    ) -> Result<DescriptorBoundInvocationRequest, AxonError> {
        let request = match ingress {
            LocalRuntimeIngress::ExternalSigned {
                envelope,
                signature,
                payload,
            } => DescriptorBoundInvocationRequest::externally_signed(
                mode, envelope, signature, payload,
            ),
        };
        Ok(Self::apply_options(request, options))
    }

    fn request_for_local_system(
        mode: AxonInvocationCallMode,
        envelope: DescriptorBoundEnvelope,
        payload: Vec<u8>,
        options: LocalRuntimeRequestOptions,
    ) -> Result<DescriptorBoundInvocationRequest, AxonError> {
        if envelope.envelope().caller.ura != LOCAL_SYSTEM_AGENT_URA {
            return Err(AxonError::invalid_argument(format!(
                "local system ingress caller must be {LOCAL_SYSTEM_AGENT_URA}, got {}",
                envelope.envelope().caller.ura
            )));
        }
        let signature = sign_system_canonical(&descriptor_bound_canonical_bytes(&envelope))
            .map_err(|error| {
                AxonError::internal(format!(
                    "sign daemon-local descriptor-bound invocation through key service: {error}"
                ))
            })?;
        let request = DescriptorBoundInvocationRequest::externally_signed(
            mode,
            envelope,
            CallerSignature {
                algorithm: "ed25519".to_string(),
                signature: signature.to_bytes().to_vec(),
                key_id_hint: String::new(),
            },
            payload,
        );
        Ok(Self::apply_options(request, options))
    }

    fn apply_options(
        mut request: DescriptorBoundInvocationRequest,
        options: LocalRuntimeRequestOptions,
    ) -> DescriptorBoundInvocationRequest {
        let trace_id = options.trace_id.trim().to_string();
        let mut request_metadata = options.request_metadata;
        request_metadata.remove(AXON_TRACE_CONTEXT_METADATA_KEY);
        if !trace_id.is_empty() {
            request_metadata.insert(
                AXON_TRACE_CONTEXT_METADATA_KEY.to_string(),
                trace_id.clone(),
            );
            request = request.with_trace_id(trace_id);
        }
        if !request_metadata.is_empty() {
            request = request.with_request_metadata(request_metadata);
        }
        request
    }
}

/// Named daemon-local issuer for `_system.local` descriptor-bound calls.
///
/// Internal daemon calls are not allowed to smuggle hidden tuple defaults
/// through individual helpers. This issuer constructs the complete seven-field
/// descriptor-bound envelope and then delegates signing/admission request
/// creation to [`LocalRuntimeRequestFactory`], so `_system.local` key custody
/// remains in one place.
pub(crate) struct SystemInvocationIssuer;

impl SystemInvocationIssuer {
    pub(crate) fn request_for_descriptor_ref(
        mode: AxonInvocationCallMode,
        callee_ura: &str,
        ability_descriptor_ref: String,
        subject_ura: &str,
        payload: Vec<u8>,
        causal_context: CausalContext,
        options: LocalRuntimeRequestOptions,
    ) -> Result<DescriptorBoundInvocationRequest, AxonError> {
        let envelope = Self::envelope_for_descriptor_ref(
            callee_ura,
            ability_descriptor_ref,
            subject_ura,
            &payload,
            causal_context,
        )?;
        Self::request_for_complete_envelope(mode, envelope, payload, options)
    }

    /// Sign one already-complete descriptor-bound envelope as `_system.local`.
    ///
    /// Wire adapters may use this only after their trusted-local transport
    /// gate has classified the request. The envelope remains fail-closed:
    /// caller, callee, subject, nonce, causal context, descriptor ref, and
    /// payload must already have been reassembled without fallback.
    pub(crate) fn request_for_complete_envelope(
        mode: AxonInvocationCallMode,
        envelope: DescriptorBoundEnvelope,
        payload: Vec<u8>,
        options: LocalRuntimeRequestOptions,
    ) -> Result<DescriptorBoundInvocationRequest, AxonError> {
        LocalRuntimeRequestFactory::request_for_local_system(mode, envelope, payload, options)
    }

    fn envelope_for_descriptor_ref(
        callee_ura: &str,
        ability_descriptor_ref: String,
        subject_ura: &str,
        payload: &[u8],
        causal_context: CausalContext,
    ) -> Result<DescriptorBoundEnvelope, AxonError> {
        let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::StrictV2);
        let subject = Self::subject_identity(subject_ura)?;
        CanonicalEnvelopeBuilder::new(
            system_agent_identity(),
            callee,
            subject,
            InvocationDerivationPolicy::FreshWithCausalContext(causal_context),
        )?
        .descriptor_bound_envelope(ability_descriptor_ref, payload)
    }

    fn subject_identity(subject_ura: &str) -> Result<SubjectIdentity, AxonError> {
        let subject = SubjectIdentity::new(subject_ura.to_string(), UraProfile::StrictV2);
        EntityRef::try_from_subject_identity(&subject).map_err(|err| {
            AxonError::invalid_argument(format!(
                "local system invocation subject `{subject_ura}` is not descriptor-bound: {err}"
            ))
        })?;
        Ok(subject)
    }
}

/// Issuer for deferred work that remains directly accountable to a User
/// Principal. The daemon may custody the managed User key, but it cannot
/// substitute `_system.local` or manufacture a User-owned Agent: the canonical
/// caller is the persisted User URA and the bound signer must match it.
pub(crate) struct AccountableUserInvocationIssuer;

impl AccountableUserInvocationIssuer {
    pub(crate) async fn request_for_descriptor_ref(
        mode: AxonInvocationCallMode,
        signer: std::sync::Arc<dyn CanonicalSigner>,
        callee_ura: &str,
        ability_descriptor_ref: String,
        subject_ura: &str,
        payload: Vec<u8>,
        causal_context: CausalContext,
        options: LocalRuntimeRequestOptions,
    ) -> Result<DescriptorBoundInvocationRequest, AxonError> {
        let caller_ura = signer.owner_ura().to_string();
        let caller = crate::core::ura::parse_ura(&caller_ura).map_err(|error| {
            AxonError::invalid_argument(format!(
                "deferred accountable caller {caller_ura:?} is invalid: {error}"
            ))
        })?;
        if caller.kind != crate::core::ura::URAKind::User || caller.user_id().is_none() {
            return Err(AxonError::invalid_argument(format!(
                "deferred accountable caller must be a canonical User Principal, got {caller_ura:?}"
            )));
        }
        let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::StrictV2);
        let subject = SystemInvocationIssuer::subject_identity(subject_ura)?;
        let envelope = CanonicalEnvelopeBuilder::new(
            AgentIdentity::new(caller_ura, UraProfile::StrictV2),
            callee,
            subject,
            InvocationDerivationPolicy::FreshWithCausalContext(causal_context),
        )?
        .descriptor_bound_envelope(ability_descriptor_ref, &payload)?;
        let signature = signer
            .sign_canonical(&descriptor_bound_canonical_bytes(&envelope))
            .await
            .map_err(|error| {
                AxonError::internal(format!(
                    "sign deferred accountable User invocation: {error}"
                ))
            })?;
        LocalRuntimeRequestFactory::request_for(
            mode,
            LocalRuntimeIngress::ExternalSigned {
                envelope,
                signature: CallerSignature {
                    algorithm: "ed25519".to_string(),
                    signature: signature.to_bytes().to_vec(),
                    key_id_hint: String::new(),
                },
                payload,
            },
            options,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::Verifier as _;

    use super::*;

    fn descriptor_ref_for(callee_ura: &str, ability: &str) -> String {
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                [0x33; 32],
                "invoke",
            )
            .expect("descriptor binding");
        crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            callee_ura,
            ability,
            &descriptor_binding,
        )
        .expect("descriptor ref")
    }

    #[tokio::test]
    async fn accountable_user_issuer_binds_user_caller_to_hosted_agent_callee() {
        let caller = crate::core::ura::user_ura("example", "alice");
        let callee = crate::core::ura::agent_ura("example", "dev-a", "assistant");
        let subject = crate::core::ura::resource_dot_ura("example", "user.alice", "job/42");
        let descriptor_ref = descriptor_ref_for(&callee, "assistant.chat");
        let payload = br#"{"prompt":"hello"}"#.to_vec();
        let signer = Arc::new(
            crate::daemon::identity::self_identity::TestCanonicalSigner::new(
                caller.clone(),
                [0x7a; 32],
            ),
        );
        let public_key = signer.signing_public_key().expect("public key");

        let request = AccountableUserInvocationIssuer::request_for_descriptor_ref(
            AxonInvocationCallMode::Rpc,
            signer,
            &callee,
            descriptor_ref.clone(),
            &subject,
            payload.clone(),
            CausalContext::None,
            LocalRuntimeRequestOptions::default(),
        )
        .await
        .expect("accountable user request");

        assert_eq!(request.call_mode(), AxonInvocationCallMode::Rpc);
        assert_eq!(request.payload(), payload.as_slice());
        let envelope = request.envelope().envelope();
        assert_eq!(envelope.caller.ura.as_str(), caller);
        assert_eq!(envelope.callee.ura.as_str(), callee);
        assert_eq!(envelope.subject.ura.as_str(), subject);
        assert_eq!(envelope.ability.as_str(), descriptor_ref);

        let signature = crate::daemon::identity::self_identity::TestCanonicalSigner::new(
            envelope.caller.ura.clone(),
            [0x7a; 32],
        )
        .sign_canonical(&request.canonical_bytes())
        .await
        .expect("canonical signature");
        public_key
            .verify(&request.canonical_bytes(), &signature)
            .expect("signature verifies against canonical User signing key");
    }

    #[tokio::test]
    async fn accountable_user_issuer_rejects_hosted_agent_signer_owner() {
        let callee = crate::core::ura::agent_ura("example", "dev-a", "assistant");
        let signer = Arc::new(
            crate::daemon::identity::self_identity::TestCanonicalSigner::new(
                callee.clone(),
                [0x5a; 32],
            ),
        );
        let err = match AccountableUserInvocationIssuer::request_for_descriptor_ref(
            AxonInvocationCallMode::Rpc,
            signer,
            &callee,
            descriptor_ref_for(&callee, "assistant.chat"),
            &crate::core::ura::resource_dot_ura("example", "user.alice", "job/42"),
            br#"{"prompt":"hello"}"#.to_vec(),
            CausalContext::None,
            LocalRuntimeRequestOptions::default(),
        )
        .await
        {
            Ok(_) => panic!("hosted Agent signer owner must not become accountable User caller"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("deferred accountable caller must be a canonical User Principal"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn accountable_user_issuer_rejects_malformed_signer_owner() {
        let callee = crate::core::ura::agent_ura("example", "dev-a", "assistant");
        let signer = Arc::new(
            crate::daemon::identity::self_identity::TestCanonicalSigner::new("alice", [0x4a; 32]),
        );
        let err = match AccountableUserInvocationIssuer::request_for_descriptor_ref(
            AxonInvocationCallMode::Rpc,
            signer,
            &callee,
            descriptor_ref_for(&callee, "assistant.chat"),
            &crate::core::ura::resource_dot_ura("example", "user.alice", "job/42"),
            br#"{"prompt":"hello"}"#.to_vec(),
            CausalContext::None,
            LocalRuntimeRequestOptions::default(),
        )
        .await
        {
            Ok(_) => panic!("malformed signer owner must fail closed before request creation"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("deferred accountable caller \"alice\" is invalid"),
            "unexpected error: {err}"
        );
    }
}
