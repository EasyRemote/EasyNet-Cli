// EasyNet CLI - daemon runtime admission policy
// ==============================================
//
// File: src/daemon/invocation/admission_facade.rs
// Description: Daemon-owned authorization and quota policy evaluated through
//              Axon's canonical receipt-provider admission seam.
//
// Boundary note
// -------------
// This module never verifies caller signatures and never owns nonce/replay
// state. Transport dispatch stages product context, Axon verifies the
// descriptor-bound invocation, and the canonical receipt provider calls this
// policy exactly once before handler execution. Quota uses a reservation so a
// later Axon replay rejection rolls the provisional count back.
//
// `_system.local` is the only unsigned classification and is accepted only
// when the service is bound to a local-only transport. Every external caller
// remains under Axon's signature, replay, lifecycle, and receipt authority.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::Digest;
use tonic::{Code, Status};

use axon_sdk::invocation::axiom::{
    authority_proof_expected_hash, AgentIdentity, KeyResolver, UraProfile,
};
use axon_sdk::invocation::{
    AbilityContext, AuthorityBinding, AuthorityEvidence, AuthorityOrBootstrap, AuthorityRelation,
    AxonError as InvocationError, AxonErrorKind as InvocationErrorKind, BootstrapBinding,
    CallMode as AxonCallMode, DelegationEvidence, DescriptorBoundEnvelope, ErrorCode, ErrorStage,
    InvocationAuthorityProof, SecurityClass, SessionEvidence, SignedEnvelope,
    VerifiedAdmissionPolicy, REASON_CALLER_SIGNATURE_INVALID, REASON_ENVELOPE_INCOMPLETE,
    REASON_NONCE_REPLAY,
};

use crate::core::ura::{parse_ura, AbilitySelector, URAKind};
use crate::daemon::ability::catalog::daemon_invocation_contracts::admission_action_for;
use crate::daemon::ability::names::{federation, governance};
use crate::daemon::ability::{
    HOSTED_AGENT_DELEGATION_METADATA_KEY, HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY,
};
use crate::daemon::axon_bridge::proof_owner::descriptor_bound_canonical_bytes;
use crate::daemon::axon_bridge::wire_descriptor::{
    descriptor_bound_from_wire_parts, WireDescriptorBoundEnvelope,
};
use crate::daemon::invocation::admission::authority_metadata::{
    self, AuthorityMetadataError, AuthoritySubjectKind, DelegationPayload, SessionAuthorityPayload,
    DELEGATION_METADATA_KEY, REASON_AUTHORITY_EXPIRED, REASON_AUTHORITY_FORMAT_INVALID,
    SESSION_AUTHORITY_METADATA_KEY,
};
use crate::daemon::invocation::admission::authority_proof::{
    request_scoped_one_time_authority_proof, AuthorityProof, AuthorityProofDenyReason,
    AuthorityProofIssuerResolver, AuthorityProofVerificationContext, AuthorityProofVerifier,
};
use crate::daemon::invocation::admission::bootstrap_authority::{
    BootstrapAuthorityDecision, BootstrapAuthorityVerifier,
};
use crate::daemon::invocation::admission::decision::{
    AccessAction, PermissionRequestStatus, PolicyDecision, SignatureDecisionReason,
};
use crate::daemon::invocation::admission::device_caller::{
    verify_device_invocation_purpose, DeviceInvocationPurposeScope, VerifiedDeviceInvocationPurpose,
};
use crate::daemon::invocation::admission::federated_key_resolver::FederatedKeyResolver;
use crate::daemon::invocation::admission::grant_matcher::{
    GrantMatchInput, PermissionEffect, PermissionGrant, PermissionGrantMatcher,
};
use crate::daemon::invocation::admission::hosted_agent_publication::HostedAgentPublication;
use crate::daemon::invocation::admission::list_user_pubkeys::ABILITY_IDENTITY_LIST_USER_PUBKEYS;
use crate::daemon::invocation::admission::local_device_resource_authority::{
    authorize_user_session_device_resource, LocalDeviceResourceAuthorityDecision,
    UserSessionDeviceResourceTuple,
};
use crate::daemon::invocation::admission::policy_gate::{
    ability_ura_for, principal_for, AdmissionPolicyContext, AdmissionPolicyGate,
    PrincipalProjection, TrustedCallerPath, VerifiedAuthorityPeerDirectoryStream,
    VerifiedCallerEvidence,
};
use crate::daemon::invocation::admission::principal_lifecycle::{
    PrincipalAdmissionState, PrincipalLifecycleReader,
};
use crate::daemon::invocation::admission::register_device_pubkey::{
    verify_user_register_pubkey_bootstrap_claim, RegisterPubkeyBootstrapTuple,
    ABILITY_IDENTITY_REGISTER_PUBKEY,
};
use crate::daemon::invocation::admission::revoke_user_pubkey::ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
use crate::daemon::invocation::admission::usage_quota::{
    QuotaDenyReason, QuotaReservation, SharedUsageQuotaGate,
};
use crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_ADVERTISE_AGENT;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    AdvertiseAgentRequest, ABILITY_FEDERATION_JOIN, ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
};
use crate::daemon::invocation::dispatch::invocation_wire::AUTHORITY_PROOF_METADATA_KEY;
use crate::daemon::persistence::access_control::{AccessControlStore, AccessControlStoreRegistry};
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustAnchorRole};
use crate::daemon::trust::cell::SharedTrustAnchor;
use axon_sdk::pb::axon::v1::Envelope;

const REASON_AUTHORITY_REQUIRED: &str = "AUTHORITY_REQUIRED";
const REASON_AUTHORITY_SIGNATURE_INVALID: &str = "AUTHORITY_SIGNATURE_INVALID";
const REASON_AUTHORITY_CALLER_MISMATCH: &str = "AUTHORITY_CALLER_MISMATCH";
const REASON_AUTHORITY_SUBJECT_MISMATCH: &str = "AUTHORITY_SUBJECT_MISMATCH";
const REASON_AUTHORITY_AUDIENCE_VIOLATION: &str = "AUTHORITY_AUDIENCE_VIOLATION";
const REASON_AUTHORITY_SCOPE_VIOLATION: &str = "AUTHORITY_SCOPE_VIOLATION";
const REASON_AUTHORITY_ISSUER_UNKNOWN: &str = "AUTHORITY_ISSUER_UNKNOWN";
const REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND: &str = "AUTHORITY_ISSUER_KEY_NOT_FOUND";
const REASON_AUTHORITY_ISSUER_DENIED: &str = "AUTHORITY_ISSUER_DENIED";
const REASON_HOSTED_AGENT_DELEGATION_LOCAL_ONLY: &str = "HOSTED_AGENT_DELEGATION_LOCAL_ONLY";

/// Per-RPC transport/runtime admission gate consulted by
/// `DaemonInvocationService` before routing into a federation wrapper or
/// fallthrough handler.
///
/// Holds:
/// - `Arc<RealmTrustAnchor>` — the trust set authored by PR-7's
///   pairing flow and read at boot by the daemon binary
/// - `daemon_ura` — the daemon's own canonical URA (local self admission)
///
/// Constructed once per daemon process; cloned into per-request
/// dispatcher tasks (clone is cheap — all fields are `Arc` or
/// `Option<String>`).
#[derive(Clone)]
pub struct AdmissionFacade {
    trust_anchor: SharedTrustAnchor,
    daemon_ura: Option<String>,
    ability_catalog: Option<Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>>,
    /// The same immutable provider instance installed in Axon's canonical
    /// admission graph. Product policy may classify federated callers and the
    /// `federation.resolve_key` handler may delegate to it, but neither owns a
    /// second signature-verification path.
    federated_keys: Option<Arc<FederatedKeyResolver>>,
    invocation_verification_keys: Option<
        Arc<dyn crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider>,
    >,
    /// Explicit transport authority for local self admission. The daemon
    /// serves the same Invocation service over local-only IPC and off-box
    /// TCP/TLS; this state prevents a caller that can reach TCP and spoof the
    /// daemon's own URA from skipping trust-anchor, signature, and replay
    /// checks.
    transport_boundary: AdmissionTransportBoundary,
    /// #185: reloadable per-consumer usage-quota gate. The gate is
    /// always present so SIGHUP can enable quota after boot; it is
    /// disabled internally when `[daemon.quota]` is absent. Local self calls
    /// remain exempt here because the daemon must not throttle its own runtime
    /// control surface.
    quota: SharedUsageQuotaGate,
    /// Process-scoped RFC-014 repository graph shared with governance
    /// abilities. Owner journals are opened once and all hash-chain mutations
    /// are serialized by this component.
    access_control_stores: Arc<AccessControlStoreRegistry>,
    /// Read-only view of the daemon-owned PrincipalLifecycle aggregate.
    /// Trust-anchor key presence proves a key is known; this reader proves
    /// whether the User principal itself is currently admissible. `None` is
    /// reserved for tests and pre-lifecycle wiring seams; production boot
    /// derives it from the same trust-anchor path used by identity writes.
    principal_lifecycle: Option<PrincipalLifecycleReader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionTransportBoundary {
    /// The facade is fed by daemon-local IPC only. The daemon's own URA and the
    /// local system URA may enter without public caller signatures.
    LocalOnlyIpc,
    /// The facade is reachable by off-box clients. Every caller, including a
    /// daemon-URA spoof, must enter the strict public admission pipeline.
    OffBoxStrict,
}

impl AdmissionTransportBoundary {
    fn admits_local_self(self) -> bool {
        matches!(self, Self::LocalOnlyIpc)
    }

    pub(crate) fn accepts_local_self_caller(
        self,
        daemon_ura: Option<&str>,
        caller_ura: &str,
    ) -> bool {
        if !self.admits_local_self() {
            return false;
        }
        caller_ura == crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
            || daemon_ura.is_some_and(|daemon_ura| daemon_ura == caller_ura)
    }
}

#[derive(Debug, Clone)]
struct BoundAdmissionDescriptor {
    owner_ura: String,
    hosted_agent_device_ura: Option<String>,
    action: AccessAction,
    safe_read: bool,
    subject_contract_ura: Option<String>,
}

#[derive(Clone)]
struct VerifiedSignedAuthority<T> {
    payload: T,
    canonical_payload: Vec<u8>,
    signature: Vec<u8>,
}

#[derive(Clone)]
struct VerifiedRuntimeAuthority {
    authority_id: Option<String>,
    session_id: Option<String>,
    binding: AuthorityOrBootstrap,
    proof_type: &'static str,
    proof_payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeAuthorityIssuerPolicy {
    RealmTrustAnchor,
    LocalSystem,
}

impl VerifiedRuntimeAuthority {
    fn self_authority(principal_ura: impl Into<String>) -> Self {
        Self {
            authority_id: None,
            session_id: None,
            binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: AgentIdentity::new(principal_ura.into(), UraProfile::StrictV2),
                relation: AuthorityRelation::Self_,
                evidence: AuthorityEvidence::Identity,
            }),
            proof_type: "self-authority",
            proof_payload: Vec::new(),
        }
    }

    // `capability`/`policy`/`trusted_local_system_capability` are all
    // daemon-internal admission facts (the daemon vouches for its own
    // structural policy evaluation, not a caller-presented cryptographic
    // claim) — they map to Bootstrap, not a signed AuthorityBinding
    // relation. See RFC 001-authority-binding-relation-evidence.md:
    // Bootstrap is the admission-plane fact for exactly this shape.
    // `capability_ura`/`policy_ura` are preserved as the audit payload
    // (proof_payload), same as before — only the structured binding
    // changed, not what's recorded for provenance.
    fn capability(
        envelope: &axon_sdk::invocation::InvocationEnvelope,
        capability_ura: &str,
    ) -> Result<Self, Status> {
        let realm = parse_ura(&envelope.callee.ura)
            .map_err(|error| {
                Status::invalid_argument(format!(
                    "capability authority callee is not a canonical URA: {error}"
                ))
            })?
            .realm;
        Ok(Self {
            authority_id: None,
            session_id: None,
            binding: AuthorityOrBootstrap::Bootstrap(BootstrapBinding {
                principal_ura: envelope.caller.ura.clone(),
                realm,
                ability: envelope.ability.clone(),
            }),
            proof_type: "causal-parent-capability",
            proof_payload: capability_ura.as_bytes().to_vec(),
        })
    }

    fn policy(
        envelope: &axon_sdk::invocation::InvocationEnvelope,
        policy_ura: &str,
        authority_id: Option<String>,
    ) -> Result<Self, Status> {
        let realm = parse_ura(&envelope.callee.ura)
            .map_err(|error| {
                Status::invalid_argument(format!(
                    "policy authority callee is not a canonical URA: {error}"
                ))
            })?
            .realm;
        Ok(Self {
            authority_id,
            session_id: None,
            binding: AuthorityOrBootstrap::Bootstrap(BootstrapBinding {
                principal_ura: envelope.caller.ura.clone(),
                realm,
                ability: envelope.ability.clone(),
            }),
            proof_type: "system-policy-authority",
            proof_payload: policy_ura.as_bytes().to_vec(),
        })
    }

    fn trusted_local_system_capability(
        envelope: &axon_sdk::invocation::InvocationEnvelope,
    ) -> Result<Self, Status> {
        let capability_ura = crate::core::ura::resource_dot_ura(
            "_system",
            "agent._system.local",
            "capability/local-system-invocation",
        );
        let realm = parse_ura(&envelope.callee.ura)
            .map_err(|error| {
                Status::invalid_argument(format!(
                    "trusted local system authority callee is not a canonical URA: {error}"
                ))
            })?
            .realm;
        Ok(Self {
            authority_id: None,
            session_id: None,
            binding: AuthorityOrBootstrap::Bootstrap(BootstrapBinding {
                principal_ura: envelope.caller.ura.clone(),
                realm,
                ability: envelope.ability.clone(),
            }),
            proof_type: "local-system-invocation-capability",
            proof_payload: capability_ura.into_bytes(),
        })
    }

    fn bootstrap(
        envelope: &axon_sdk::invocation::InvocationEnvelope,
        authority_id: Option<String>,
    ) -> Result<Self, Status> {
        let realm = parse_ura(&envelope.callee.ura)
            .map_err(|error| {
                Status::invalid_argument(format!(
                    "bootstrap authority callee is not a canonical URA: {error}"
                ))
            })?
            .realm;
        Ok(Self {
            authority_id,
            session_id: None,
            binding: AuthorityOrBootstrap::Bootstrap(BootstrapBinding {
                principal_ura: envelope.caller.ura.clone(),
                realm,
                ability: envelope.ability.clone(),
            }),
            proof_type: "bootstrap-authority",
            proof_payload: Vec::new(),
        })
    }

    fn delegated(verified: VerifiedSignedAuthority<DelegationPayload>) -> Result<Self, Status> {
        let authority_id = verified_delegation_authority_id(&verified.payload)?;
        // Field provenance (RFC 001-authority-binding-relation-evidence.md
        // "Field provenance" section): the old `subject_ura` ("who the
        // caller acts for") maps to the outer AuthorityBinding.authority
        // field, NOT to any subject slot. `issuer_ura` maps to
        // evidence.issuer — a distinct role, MAY differ from authority
        // (see "Issuer authenticity vs. issuer authority"). The old
        // `caller_ura` is dropped entirely (redundant with
        // envelope.caller): the SDK's signature verification binds
        // envelope.caller as delegatee directly into the signed claim
        // bytes instead of storing it as a plain field.
        Ok(Self {
            authority_id: Some(authority_id),
            session_id: None,
            binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: AgentIdentity::new(verified.payload.subject_ura, UraProfile::StrictV2),
                relation: AuthorityRelation::DelegatedBy,
                evidence: AuthorityEvidence::Delegation(DelegationEvidence {
                    issuer: AgentIdentity::new(verified.payload.issuer_ura, UraProfile::StrictV2),
                    scopes: verified.payload.scopes,
                    audience: verified.payload.audience,
                    issued_at_ms: verified.payload.issued_at_ms,
                    expires_at_ms: verified.payload.expires_at_ms,
                    signature: verified.signature,
                }),
            }),
            proof_type: "delegated-authority",
            proof_payload: verified.canonical_payload,
        })
    }

    fn session(verified: VerifiedSignedAuthority<SessionAuthorityPayload>) -> Result<Self, Status> {
        // Structural sanity check only — subject_ura must be a canonical
        // URA. Do NOT derive a plain user_ura from it: envelope.subject is
        // constrained by the SDK's DescriptorBoundEnvelope to
        // Agent/Service/Authority/Ability/Device/Resource kinds and can
        // never be a bare User URA (confirmed: EntityRefKind has no User
        // variant). The daemon's own pre-existing
        // verify_session_authority_bindings (via
        // authority_metadata::session_authority_admits_subject) already
        // requires `payload.subject_ura == envelope.subject` as a raw
        // string equality — so binding.authority must be
        // verified.payload.subject_ura itself, not a re-derived value.
        parse_ura(&verified.payload.subject_ura).map_err(|error| {
            Status::invalid_argument(format!(
                "session authority subject is not a canonical URA: {error}"
            ))
        })?;
        Ok(Self {
            authority_id: Some(verified_session_authority_id(&verified.payload)),
            session_id: Some(verified.payload.session_id.clone()),
            // Field provenance: the old `subject_ura` genuinely always
            // meant envelope.subject (Bucket A archaeology — confirmed
            // unconditional from introduction through the field rename)
            // and maps to the outer AuthorityBinding.authority field.
            binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: AgentIdentity::new(
                    verified.payload.subject_ura.clone(),
                    UraProfile::StrictV2,
                ),
                relation: AuthorityRelation::SessionOf,
                evidence: AuthorityEvidence::Session(SessionEvidence {
                    issuer: AgentIdentity::new(verified.payload.issuer_ura, UraProfile::StrictV2),
                    session_id: verified.payload.session_id,
                    scopes: verified.payload.scopes,
                    audiences: vec![verified.payload.audience],
                    issued_at_ms: verified.payload.issued_at_ms,
                    expires_at_ms: verified.payload.expires_at_ms,
                    signature: verified.signature,
                }),
            }),
            proof_type: "session-authority",
            proof_payload: verified.canonical_payload,
        })
    }

    fn from_authority_proof(proof: &AuthorityProof) -> Result<Self, Status> {
        let issued_at_ms = DateTime::parse_from_rfc3339(&proof.issued_at)
            .map_err(|error| {
                Status::invalid_argument(format!(
                    "AUTHORITY_PROOF_MISMATCH: issued_at is not RFC3339: {error}"
                ))
            })?
            .timestamp_millis();
        let expires_at_ms = DateTime::parse_from_rfc3339(&proof.expires_at)
            .map_err(|error| {
                Status::invalid_argument(format!(
                    "AUTHORITY_PROOF_MISMATCH: expires_at is not RFC3339: {error}"
                ))
            })?
            .timestamp_millis();
        let encoded_signature = proof
            .signature
            .trim()
            .strip_prefix("ed25519:")
            .unwrap_or_else(|| proof.signature.trim());
        let signature = BASE64_STANDARD.decode(encoded_signature).map_err(|error| {
            Status::invalid_argument(format!(
                "AUTHORITY_PROOF_MISMATCH: signature is not base64: {error}"
            ))
        })?;
        Ok(Self {
            authority_id: Some(proof.proof_id.clone()),
            session_id: proof.session_id.clone(),
            // Same field provenance as delegated() above: subject_ura ->
            // authority, issuer_ura -> evidence.issuer, caller_ura dropped
            // (redundant with envelope.caller, bound into the signed
            // claim bytes instead of a stored field).
            binding: AuthorityOrBootstrap::Binding(AuthorityBinding {
                authority: AgentIdentity::new(proof.subject_ura.clone(), UraProfile::StrictV2),
                relation: AuthorityRelation::DelegatedBy,
                evidence: AuthorityEvidence::Delegation(DelegationEvidence {
                    issuer: AgentIdentity::new(proof.issuer_ura.clone(), UraProfile::StrictV2),
                    scopes: vec![proof.ability_ura.clone()],
                    audience: proof.audience_ura.clone(),
                    issued_at_ms,
                    expires_at_ms,
                    signature,
                }),
            }),
            proof_type: "delegated-authority",
            proof_payload: crate::daemon::ability::canonical_json_bytes(
                &proof.canonical_material(),
            ),
        })
    }

    fn authority_id(&self) -> Option<&str> {
        self.authority_id.as_deref()
    }

    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn with_policy_decision(mut self, decision: &PolicyDecision) -> Result<Self, Status> {
        self.proof_payload = runtime_admission_policy_proof_payload(
            self.proof_type,
            &self.proof_payload,
            Some(decision),
            None,
        )?;
        Ok(self)
    }

    fn with_runtime_admission_fact(mut self, reason: &'static str) -> Result<Self, Status> {
        self.proof_payload = runtime_admission_policy_proof_payload(
            self.proof_type,
            &self.proof_payload,
            None,
            Some(reason),
        )?;
        Ok(self)
    }

    fn authority_proof(&self, envelope: &DescriptorBoundEnvelope) -> InvocationAuthorityProof {
        let mut proof = InvocationAuthorityProof::new(
            self.proof_type,
            Some(self.binding.clone()),
            self.proof_payload.clone(),
            [0u8; 32],
            Some(envelope.envelope().callee.clone()),
            None,
            "runtime.admission.v1",
        );
        proof.proof_hash = authority_proof_expected_hash(&proof);
        proof
    }

    fn into_policy(
        self,
        envelope: &DescriptorBoundEnvelope,
    ) -> Result<VerifiedAdmissionPolicy, InvocationError> {
        let proof = self.authority_proof(envelope);
        VerifiedAdmissionPolicy::new(envelope, self.binding, proof)
    }
}

fn runtime_admission_policy_proof_payload(
    proof_type: &str,
    authority_payload: &[u8],
    policy_decision: Option<&PolicyDecision>,
    bootstrap_reason: Option<&'static str>,
) -> Result<Vec<u8>, Status> {
    let authority_payload_hash = format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(authority_payload))
    );
    let mut value = serde_json::json!({
        "profile": "easynet-runtime-admission-proof-v1",
        "authority": {
            "proof_type": proof_type,
            "payload_base64": BASE64_STANDARD.encode(authority_payload),
            "payload_hash": authority_payload_hash,
        }
    });
    if let Some(decision) = policy_decision {
        let decision_value = serde_json::to_value(decision).map_err(|error| {
            Status::internal(format!(
                "POLICY_PROOF_SERIALIZATION_FAILED: policy decision could not serialize: {error}"
            ))
        })?;
        let decision_bytes = crate::daemon::ability::canonical_json_bytes(&decision_value);
        value["policy_decision"] = decision_value;
        value["policy_decision_hash"] = serde_json::json!(format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(decision_bytes))
        ));
    } else if let Some(reason) = bootstrap_reason {
        value["bootstrap_admission"] = serde_json::json!({
            "reason": reason,
        });
    }
    Ok(crate::daemon::ability::canonical_json_bytes(&value))
}

const MAX_PENDING_RUNTIME_ADMISSIONS: usize = 4_096;

#[derive(Clone, Debug, PartialEq, Eq)]
enum RuntimeAdmissionIngress {
    CallerSigned,
    BootstrapCandidate,
    TrustedLocalSystem,
    DerivedChild(DerivedRuntimeAuthorityContext),
}

/// Runtime-proven authority inherited by a composite-ability child.
///
/// The child envelope still names its executing SystemAgent/Agent as caller.
/// This context only carries the parent authority fact and exact causal
/// receipt that prove which principal is accountable for that execution.
/// It is constructed from Axon's admitted [`AbilityContext`], never from
/// public metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DerivedRuntimeAuthorityContext {
    parent_authority: AuthorityOrBootstrap,
    capability_ura: String,
}

impl DerivedRuntimeAuthorityContext {
    fn from_parent(parent: &AbilityContext, child: &SignedEnvelope) -> Result<Self, Status> {
        let parent_envelope = parent.signed_envelope().ok_or_else(|| {
            Status::internal("derived child admission parent has no signed envelope")
        })?;
        if child.envelope.caller != parent_envelope.envelope.callee {
            return Err(Status::permission_denied(
                "derived child caller does not match admitted parent callee",
            ));
        }
        let capability_ura = match &child.envelope.causal_context {
            axon_sdk::invocation::CausalContext::Scalar(anchor) => anchor.receipt_ura.clone(),
            _ => {
                return Err(Status::permission_denied(
                    "derived child admission requires the exact scalar parent receipt",
                ))
            }
        };
        if capability_ura.trim().is_empty() {
            return Err(Status::permission_denied(
                "derived child parent receipt URA is empty",
            ));
        }
        Ok(Self {
            parent_authority: parent.authority_binding().clone(),
            capability_ura,
        })
    }

    fn accountable_user_ura(&self) -> Option<&str> {
        fn canonical_user(candidate: &str) -> Option<&str> {
            parse_ura(candidate)
                .ok()
                .filter(|parsed| parsed.kind == URAKind::User)
                .map(|_| candidate)
        }
        let AuthorityOrBootstrap::Binding(binding) = &self.parent_authority else {
            // Bootstrap is a daemon-internal admission fact, not a
            // caller-presented identity claim — no accountable user to
            // extract (matches the old Capability/Policy/Bootstrap arms,
            // all of which now map to Bootstrap).
            return None;
        };
        match (&binding.relation, &binding.evidence) {
            (AuthorityRelation::Self_, AuthorityEvidence::Identity) => {
                canonical_user(&binding.authority.ura)
            }
            (AuthorityRelation::DelegatedBy, AuthorityEvidence::Delegation(evidence)) => {
                canonical_user(&binding.authority.ura)
                    .or_else(|| canonical_user(&evidence.issuer.ura))
            }
            (AuthorityRelation::SessionOf, AuthorityEvidence::Session(_)) => {
                canonical_user(&binding.authority.ura)
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
struct RuntimeAdmissionInput {
    facade: AdmissionFacade,
    envelope: Envelope,
    ability: String,
    arguments: Vec<u8>,
    metadata: HashMap<String, String>,
    call_mode: AxonCallMode,
    ingress: RuntimeAdmissionIngress,
}

struct RuntimeAdmissionReservation {
    quota: Option<QuotaReservation>,
}

struct RuntimeAdmissionDecision {
    reservation: RuntimeAdmissionReservation,
    policy: VerifiedAdmissionPolicy,
}

enum RuntimeAdmissionState {
    Pending,
    Evaluating,
    Verified(RuntimeAdmissionReservation),
    Denied,
}

struct PendingRuntimeAdmission {
    id: u64,
    input: RuntimeAdmissionInput,
    state: RuntimeAdmissionState,
}

#[derive(Default)]
struct RuntimeAdmissionRegistry {
    by_envelope: HashMap<[u8; 32], VecDeque<PendingRuntimeAdmission>>,
    len: usize,
}

/// Request-scoped bridge from daemon transport context to Axon's synchronous
/// canonical receipt-provider admission seam.
///
/// The registry carries runtime-only facts that are not part of the canonical
/// descriptor-bound envelope, such as request metadata and quota policy. It
/// never verifies signatures and never stores nonce/replay state. Axon remains
/// the sole owner of both decisions.
#[derive(Default)]
pub(crate) struct DaemonRuntimeAdmissionCoordinator {
    registry: Mutex<RuntimeAdmissionRegistry>,
    next_id: AtomicU64,
}

impl DaemonRuntimeAdmissionCoordinator {
    pub(crate) fn stage(
        self: &Arc<Self>,
        facade: &AdmissionFacade,
        wire: &crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch,
        ability: &str,
        call_mode: AxonCallMode,
    ) -> Result<DaemonRuntimeAdmissionLease, Status> {
        let caller_signature = match &wire.ingress {
            crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::ExternalSigned(_) => {
                Some(wire_caller_signature(wire)?)
            }
            crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::BootstrapCandidate(
                _,
            ) => Some(wire_caller_signature(wire)?),
            crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::LocalSystem => {
                None
            }
        };
        let ingress = match &wire.ingress {
            crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::ExternalSigned(_) => {
                RuntimeAdmissionIngress::CallerSigned
            }
            crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::BootstrapCandidate(
                _,
            ) => RuntimeAdmissionIngress::BootstrapCandidate,
            crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::LocalSystem => {
                RuntimeAdmissionIngress::TrustedLocalSystem
            }
        };
        self.stage_canonical(
            facade,
            &wire.envelope,
            caller_signature,
            wire.payload.clone(),
            wire.request_metadata.clone(),
            wire.trace_id.clone(),
            ability,
            call_mode,
            ingress,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "staging consumes the complete canonical ingress tuple and does not permit defaulted security facts"
    )]
    fn stage_canonical(
        self: &Arc<Self>,
        facade: &AdmissionFacade,
        descriptor_bound: &DescriptorBoundEnvelope,
        caller_signature: Option<axon_sdk::invocation::CallerSignature>,
        arguments: Vec<u8>,
        metadata: HashMap<String, String>,
        request_id: String,
        ability: &str,
        call_mode: AxonCallMode,
        ingress: RuntimeAdmissionIngress,
    ) -> Result<DaemonRuntimeAdmissionLease, Status> {
        let envelope_key =
            axon_sdk::invocation::sha256(&descriptor_bound_canonical_bytes(descriptor_bound));
        let envelope =
            runtime_admission_envelope(descriptor_bound.envelope(), caller_signature, request_id)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut registry = self.registry.lock().map_err(|_| {
            Status::internal("daemon runtime admission registry lock poisoned while staging")
        })?;
        if registry.len >= MAX_PENDING_RUNTIME_ADMISSIONS {
            return Err(Status::resource_exhausted(
                "daemon runtime admission registry is saturated",
            ));
        }
        registry
            .by_envelope
            .entry(envelope_key)
            .or_default()
            .push_back(PendingRuntimeAdmission {
                id,
                input: RuntimeAdmissionInput {
                    facade: facade.clone(),
                    envelope,
                    ability: ability.to_string(),
                    arguments,
                    metadata,
                    call_mode,
                    ingress,
                },
                state: RuntimeAdmissionState::Pending,
            });
        registry.len += 1;
        Ok(DaemonRuntimeAdmissionLease {
            coordinator: Arc::clone(self),
            envelope_key,
            id: Some(id),
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "derived staging binds the complete signed child tuple and does not permit defaulted security facts"
    )]
    fn stage_derived(
        self: &Arc<Self>,
        facade: &AdmissionFacade,
        parent: &AbilityContext,
        descriptor_bound: &DescriptorBoundEnvelope,
        signed_envelope: &axon_sdk::invocation::SignedEnvelope,
        arguments: Vec<u8>,
        metadata: HashMap<String, String>,
        request_id: String,
        ability: &str,
        call_mode: AxonCallMode,
    ) -> Result<DaemonRuntimeAdmissionLease, Status> {
        let signed_descriptor_bound =
            DescriptorBoundEnvelope::new(signed_envelope.envelope.clone()).map_err(|error| {
                Status::invalid_argument(format!(
                    "derived runtime admission signed envelope is not descriptor-bound: {error}"
                ))
            })?;
        if descriptor_bound_canonical_bytes(descriptor_bound)
            != descriptor_bound_canonical_bytes(&signed_descriptor_bound)
        {
            return Err(Status::invalid_argument(
                "derived runtime admission signed envelope does not match descriptor-bound request",
            ));
        }
        let derived_authority =
            DerivedRuntimeAuthorityContext::from_parent(parent, signed_envelope)?;
        self.stage_canonical(
            facade,
            descriptor_bound,
            Some(signed_envelope.signature.clone()),
            arguments,
            metadata,
            request_id,
            ability,
            call_mode,
            RuntimeAdmissionIngress::DerivedChild(derived_authority),
        )
    }

    pub(crate) fn verify_provider_policy(
        &self,
        envelope: &DescriptorBoundEnvelope,
    ) -> Result<VerifiedAdmissionPolicy, InvocationError> {
        let envelope_key =
            axon_sdk::invocation::sha256(&descriptor_bound_canonical_bytes(envelope));
        let selected = {
            let mut registry = self.registry.lock().map_err(|_| {
                InvocationError::internal(
                    "daemon_runtime_admission_registry_lock_poisoned_while_verifying",
                )
            })?;
            let Some(queue) = registry.by_envelope.get_mut(&envelope_key) else {
                return Err(InvocationError::permission_denied(
                    "daemon_runtime_admission_context_missing",
                ));
            };
            let pending = queue
                .iter_mut()
                .find(|pending| matches!(pending.state, RuntimeAdmissionState::Pending))
                .ok_or_else(|| {
                    InvocationError::permission_denied(
                        "daemon_runtime_admission_context_not_pending",
                    )
                })?;
            pending.state = RuntimeAdmissionState::Evaluating;
            (pending.id, pending.input.clone())
        };

        let (id, input) = selected;
        let result = input
            .facade
            .reserve_runtime_admission(&input, envelope)
            .map_err(runtime_admission_status_to_axon);

        let mut registry = self.registry.lock().map_err(|_| {
            InvocationError::internal(
                "daemon_runtime_admission_registry_lock_poisoned_after_verification",
            )
        })?;
        let queue = registry.by_envelope.get_mut(&envelope_key).ok_or_else(|| {
            InvocationError::internal("daemon_runtime_admission_context_removed_while_verifying")
        })?;
        let pending = queue
            .iter_mut()
            .find(|pending| pending.id == id)
            .ok_or_else(|| {
                InvocationError::internal(
                    "daemon_runtime_admission_context_id_removed_while_verifying",
                )
            })?;
        match result {
            Ok(decision) => {
                pending.state = RuntimeAdmissionState::Verified(decision.reservation);
                Ok(decision.policy)
            }
            Err(error) => {
                pending.state = RuntimeAdmissionState::Denied;
                Err(error)
            }
        }
    }

    fn finish(&self, envelope_key: [u8; 32], id: u64, commit: bool) -> Result<(), Status> {
        let state = {
            let mut registry = self.registry.lock().map_err(|_| {
                Status::internal("daemon runtime admission registry lock poisoned while finishing")
            })?;
            let (state, remove_key) = {
                let queue = registry.by_envelope.get_mut(&envelope_key).ok_or_else(|| {
                    Status::internal("daemon runtime admission lease has no staged context")
                })?;
                let offset = queue
                    .iter()
                    .position(|pending| pending.id == id)
                    .ok_or_else(|| {
                        Status::internal("daemon runtime admission lease id is not staged")
                    })?;
                let pending = queue
                    .remove(offset)
                    .expect("located daemon runtime admission must remain in queue");
                (pending.state, queue.is_empty())
            };
            registry.len = registry.len.saturating_sub(1);
            if remove_key {
                registry.by_envelope.remove(&envelope_key);
            }
            state
        };

        match (commit, state) {
            (true, RuntimeAdmissionState::Verified(reservation)) => {
                if let Some(quota) = reservation.quota {
                    quota.commit();
                }
                Ok(())
            }
            (false, _) => Ok(()),
            (true, RuntimeAdmissionState::Denied) => Err(Status::permission_denied(
                "daemon runtime admission was denied before runtime launch",
            )),
            (true, RuntimeAdmissionState::Pending | RuntimeAdmissionState::Evaluating) => {
                Err(Status::internal(
                    "daemon runtime admission did not reach a terminal policy decision",
                ))
            }
        }
    }
}

/// Owns one staged runtime-admission transaction until LocalRuntime either
/// returns an admitted handle or rejects before handler execution.
pub(crate) struct DaemonRuntimeAdmissionLease {
    coordinator: Arc<DaemonRuntimeAdmissionCoordinator>,
    envelope_key: [u8; 32],
    id: Option<u64>,
}

impl DaemonRuntimeAdmissionLease {
    pub(crate) fn commit(mut self) -> Result<(), Status> {
        let id = self
            .id
            .take()
            .ok_or_else(|| Status::internal("daemon runtime admission lease already finished"))?;
        self.coordinator.finish(self.envelope_key, id, true)
    }
}

impl Drop for DaemonRuntimeAdmissionLease {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = self.coordinator.finish(self.envelope_key, id, false);
        }
    }
}

struct AuthorityProofMetadataInput<'a> {
    envelope: &'a Envelope,
    ability: &'a str,
    action: AccessAction,
    metadata: Option<&'a HashMap<String, String>>,
    trust_anchor: &'a RealmTrustAnchor,
    trusted_path: TrustedCallerPath,
    descriptor_bound: &'a WireDescriptorBoundEnvelope,
}

impl std::fmt::Debug for AdmissionFacade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionFacade")
            .field("trust_anchor", &self.trust_anchor)
            .field("daemon_ura", &self.daemon_ura)
            .field(
                "federated_keys",
                &self
                    .federated_keys
                    .as_ref()
                    .map(|_| "<FederatedKeyResolver>"),
            )
            .field("transport_boundary", &self.transport_boundary)
            .field("quota_configured", &self.quota.policy().is_some())
            .field("access_control_stores", &"<AccessControlStoreRegistry>")
            .field(
                "principal_lifecycle",
                &self.principal_lifecycle.as_ref().map(|_| "wired"),
            )
            .finish()
    }
}

impl AdmissionFacade {
    /// Construct a facade against the supplied trust anchor and
    /// daemon URA. Production callers thread the daemon's
    /// `credentials.json`-derived URA through; tests typically pass
    /// `None`. AuthorityProof-bearing admission is stricter: the proof
    /// audience is the daemon URA and therefore requires this field to be
    /// present instead of inferring an audience from the selected callee.
    ///
    /// The trust anchor is wrapped in a fresh `SharedTrustAnchor`
    /// cell — every `verify_*` call snapshots the current anchor,
    /// so a future writer (`identity.register_pubkey`,
    /// PR-7 commit 5/N) that holds a clone of the cell can publish
    /// updates without restarting the facade. Callers that already
    /// hold a `SharedTrustAnchor` and need to share it with the
    /// register handler should use `with_trust_anchor_cell` instead.
    ///
    #[must_use]
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>, daemon_ura: Option<String>) -> Self {
        Self::with_trust_anchor_cell(SharedTrustAnchor::new(trust_anchor), daemon_ura)
    }

    /// Construct a facade against a shared trust-anchor cell. Used
    /// by `start_daemon_invocation_transport` so the same cell is shared
    /// with the `identity.register_pubkey` handler — a
    /// successful register publishes the new anchor and the next
    /// admission snapshot reflects it without daemon restart.
    #[must_use]
    pub fn with_trust_anchor_cell(
        trust_anchor: SharedTrustAnchor,
        daemon_ura: Option<String>,
    ) -> Self {
        Self {
            trust_anchor,
            daemon_ura,
            ability_catalog: None,
            federated_keys: None,
            invocation_verification_keys: None,
            // Embedders and tests are strict by default. Only the daemon boot
            // path that owns an authenticated local IPC listener may opt into
            // local-self admission explicitly.
            transport_boundary: AdmissionTransportBoundary::OffBoxStrict,
            quota: SharedUsageQuotaGate::disabled(),
            access_control_stores: default_access_control_stores(),
            principal_lifecycle: None,
        }
    }

    /// Bind runtime admission to the same policy-store registry used by the
    /// governance ability catalog.
    #[must_use]
    pub fn with_access_control_stores(mut self, stores: Arc<AccessControlStoreRegistry>) -> Self {
        self.access_control_stores = stores;
        self
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn access_control_stores(&self) -> Arc<AccessControlStoreRegistry> {
        Arc::clone(&self.access_control_stores)
    }

    /// Bind admission to the same live descriptor catalog used for dispatch.
    #[must_use]
    pub fn with_ability_catalog(
        mut self,
        catalog: Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>,
    ) -> Self {
        self.ability_catalog = Some(catalog);
        self
    }

    fn bound_admission_descriptor(
        &self,
        ability: &str,
        call_mode: crate::daemon::ability::CallMode,
        descriptor_ref: &str,
    ) -> Result<BoundAdmissionDescriptor, Status> {
        let catalog = self.ability_catalog.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "ADMISSION_DESCRIPTOR_CATALOG_UNAVAILABLE: admission has no live ability catalog",
            )
        })?;
        let signed_ability_ura =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                descriptor_ref,
            )
            .map_err(axon_error_to_status)?;
        let selector = AbilitySelector::parse(&signed_ability_ura).map_err(|error| {
            Status::invalid_argument(format!(
                "ADMISSION_DESCRIPTOR_ROUTE_MISMATCH: signed descriptor ability `{signed_ability_ura}` is not canonical: {error}"
            ))
        })?;
        let public_ability =
            public_ability_name_from_route_for_owner(selector.owner_ura(), ability);
        if selector.public_name() != public_ability {
            return Err(Status::invalid_argument(format!(
                "ADMISSION_DESCRIPTOR_ROUTE_MISMATCH: signed descriptor public ability `{}` does not match bound public route `{public_ability}`",
                selector.public_name()
            )));
        }
        let owner = crate::daemon::axon_bridge::descriptor_ref::catalog_owner_kind_for_wire(
            selector.owner_ura(),
        )
        .map_err(axon_error_to_status)?;
        let descriptor = catalog
            .public_descriptor_for_mode(&owner, &public_ability, call_mode)
            .map_err(|error| match error {
                crate::daemon::ability::dispatch::PublicDescriptorLookupError::Missing {
                    ..
                } => Status::failed_precondition(format!(
                    "ADMISSION_DESCRIPTOR_MISSING: no bound descriptor for owner {:?} public ability {public_ability:?} {call_mode:?}",
                    owner
                )),
                crate::daemon::ability::dispatch::PublicDescriptorLookupError::Ambiguous {
                    ..
                } => Status::failed_precondition(format!(
                    "ADMISSION_DESCRIPTOR_AMBIGUOUS: owner {:?} public ability {public_ability:?} {call_mode:?}",
                    owner
                )),
            })?
            .rebind_owner_ura(selector.owner_ura())
            .map_err(|error| {
                Status::failed_precondition(format!(
                    "ADMISSION_DESCRIPTOR_BINDING_INVALID: bound {public_ability:?} descriptor cannot bind to owner `{}`: {error}",
                    selector.owner_ura()
                ))
            })?;
        let signed_version =
            axon_sdk::invocation::descriptor_version_from_descriptor_ref(descriptor_ref)
                .map_err(axon_error_to_status)?;
        let signed_hash = axon_sdk::invocation::descriptor_hash_from_descriptor_ref(descriptor_ref)
            .map_err(axon_error_to_status)?;
        let signed_action =
            axon_sdk::invocation::admission_action_from_descriptor_ref(descriptor_ref)
                .map_err(axon_error_to_status)?;
        if signed_version != descriptor.version
            || signed_hash != descriptor.descriptor_hash_bytes()
            || signed_action != descriptor.admission_action().as_str()
        {
            return Err(Status::failed_precondition(format!(
                "ADMISSION_DESCRIPTOR_BINDING_MISMATCH: signed descriptor does not match the bound {public_ability:?} contract"
            )));
        }
        let action = descriptor.admission_action().into();
        let parsed_owner = parse_ura(selector.owner_ura()).map_err(|error| {
            Status::failed_precondition(format!("ADMISSION_DESCRIPTOR_OWNER_INVALID: {error}"))
        })?;
        let hosted_agent_device_ura = if parsed_owner.kind == URAKind::Agent
            && parsed_owner.agent_ids().is_some()
            && parsed_owner.device_agent_ids().is_none()
        {
            Some(
                catalog
                    .exact_hosted_agent_device_authority_root(selector.owner_ura())
                    .ok_or_else(|| {
                        Status::permission_denied(format!(
                            "HOSTED_AGENT_NOT_PUBLISHED: Agent owner `{}` is not an exact local hosted authority",
                            selector.owner_ura()
                        ))
                    })?
                    .to_string(),
            )
        } else {
            None
        };
        Ok(BoundAdmissionDescriptor {
            owner_ura: selector.owner_ura().to_string(),
            hosted_agent_device_ura,
            action,
            safe_read: action == AccessAction::Read,
            subject_contract_ura: descriptor.metadata.get("subject_contract_ura").cloned(),
        })
    }

    /// Snapshot the SharedTrustAnchor cell. PR-N2 commit 2/N's
    /// `federation.resolve_key` handler consults this at dispatch
    /// time so a SIGHUP-driven `realm-trust.toml` reload (PR-7
    /// commit 5/N) is visible without restart. Returns the
    /// current `Arc<RealmTrustAnchor>`; callers pass it directly
    /// to `federation_wrappers::handle_resolve_key`.
    #[must_use]
    pub fn trust_anchor_snapshot(&self) -> Arc<RealmTrustAnchor> {
        self.trust_anchor.snapshot()
    }

    /// Build the canonical receipt verifier over the same hot-reload trust
    /// authority used by strict invocation admission. Forwarding adapters use
    /// this to authenticate remote callee/host receipt signatures; they must
    /// not construct a transport-local key table.
    pub(crate) fn receipt_key_resolver(&self) -> Arc<dyn axon_sdk::invocation::KeyResolver> {
        if let Some(resolver) = self.federated_keys.as_ref() {
            let resolver: Arc<dyn axon_sdk::invocation::KeyResolver> = resolver.clone();
            return resolver;
        }
        Arc::new(
            crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver::new(
                self.trust_anchor.clone(),
            ),
        )
    }

    /// Resolve a caller public key through the same canonical resolver inputs
    /// used by strict Axon admission, returning base64-encoded Ed25519 bytes
    /// for the `federation.resolve_key` wire surface.
    ///
    /// Local trust-anchor hits are handled by `federation_wrappers` before this
    /// method is called. Same-realm User misses then consult the durable
    /// PrincipalLifecycle aggregate, because the trust anchor is a key
    /// projection while PrincipalLifecycle owns the canonical principal/key
    /// lifecycle. Only external callers continue into the explicit-peer-gated
    /// federated resolver.
    pub fn resolve_federated_key_b64(
        &self,
        agent_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> Result<Option<String>, Status> {
        let Some(resolver) = self.federated_keys.as_ref() else {
            return Ok(None);
        };
        let resolver = presented_pubkey_b64
            .filter(|value| !value.is_empty())
            .map_or_else(
                || Arc::clone(resolver),
                |pubkey| {
                    Arc::new(resolver.request_scoped_with_presented_pubkey_b64(pubkey.to_string()))
                },
            );
        match resolver.resolve(agent_ura) {
            Ok(key) => {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = federated_resolve_key_succeeded,
                    agent_ura = agent_ura,
                );
                Ok(Some(BASE64_STANDARD.encode(key.to_bytes())))
            }
            Err(err) if err.reason == "CALLER_KEY_NOT_FOUND" => {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = federated_resolve_key_not_found,
                    agent_ura = agent_ura,
                    detail = err.message.as_str(),
                );
                Ok(None)
            }
            Err(err) => Err(axon_error_to_status(err)),
        }
    }

    /// The daemon's own canonical URA. Used by per-ability
    /// admission filters that need to recognise the local self
    /// caller (eg. `federation.list_user_devices` in N3-5
    /// admits the daemon talking to itself without requiring
    /// a Hub trust entry for its own URA).
    #[must_use]
    pub fn daemon_ura(&self) -> Option<&str> {
        self.daemon_ura.as_deref()
    }

    fn authority_proof_audience_ura(&self) -> Result<&str, Status> {
        self.daemon_ura
            .as_deref()
            .map(str::trim)
            .filter(|daemon_ura| !daemon_ura.is_empty())
            .ok_or_else(|| {
                Status::failed_precondition(
                    "AUTHORITY_PROOF_AUDIENCE_MISSING: AuthorityProof admission requires \
                     a daemon URA audience; refusing to infer proof audience from callee",
                )
            })
    }

    /// Set the transport boundary that governs local self admission. Facades
    /// default to `OffBoxStrict`; boot explicitly opts only the authenticated
    /// local-IPC service clone into `LocalOnlyIpc`, so an off-box caller that
    /// spoofs the daemon URA cannot skip trust, signature, or replay checks.
    #[must_use]
    pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.transport_boundary = boundary;
        self
    }

    #[must_use]
    pub(crate) fn transport_boundary(&self) -> AdmissionTransportBoundary {
        self.transport_boundary
    }

    /// #185: attach the reloadable per-consumer usage-quota gate.
    /// Boot wires the same gate into the SIGHUP reload coordinator so
    /// `[daemon.quota]` edits can affect the next admission without a
    /// daemon restart.
    #[must_use]
    pub fn with_quota_gate(mut self, gate: SharedUsageQuotaGate) -> Self {
        self.quota = gate;
        self
    }

    /// Wire PrincipalLifecycle admission-state enforcement. This is read-only:
    /// lifecycle writes still happen through `principal.lifecycle.*` abilities
    /// and RuntimeTrust remains the single key projection used by signature
    /// verification.
    #[must_use]
    pub(crate) fn with_principal_lifecycle_reader(
        mut self,
        reader: PrincipalLifecycleReader,
    ) -> Self {
        self.principal_lifecycle = Some(reader);
        self
    }

    fn reserve_runtime_admission(
        &self,
        input: &RuntimeAdmissionInput,
        admitted_envelope: &DescriptorBoundEnvelope,
    ) -> Result<RuntimeAdmissionDecision, Status> {
        let descriptor_ref = admitted_envelope.envelope().ability.as_str();
        ensure_signed_descriptor_ref_matches_route(&input.envelope, &input.ability, descriptor_ref)
            .map_err(|status| {
                self.signature_denied_status(&input.envelope, &input.ability, status)
            })?;
        let descriptor_bound = descriptor_bound_from_wire_parts(
            input.envelope.clone(),
            descriptor_ref.to_string(),
            &input.arguments,
        )
        .map_err(axon_error_to_status)
        .map_err(|status| self.signature_denied_status(&input.envelope, &input.ability, status))?;
        if descriptor_bound_canonical_bytes(&descriptor_bound.envelope)
            != descriptor_bound_canonical_bytes(admitted_envelope)
        {
            return Err(self.signature_denied_status(
                &input.envelope,
                &input.ability,
                Status::permission_denied(
                    "ADMISSION_DESCRIPTOR_BINDING_MISMATCH: staged product context does not bind the runtime envelope",
                ),
            ));
        }
        let descriptor = self.bound_admission_descriptor(
            &input.ability,
            daemon_call_mode(input.call_mode),
            descriptor_ref,
        )?;
        reject_host_local_permission_probe_target_resource_subject(
            &descriptor,
            &input.envelope,
            &input.ability,
        )?;
        require_local_hosted_agent_publication_ready(
            &descriptor.owner_ura,
            descriptor.hosted_agent_device_ura.as_deref(),
        )?;
        let caller_ura = caller_ura_required(&input.envelope)?;

        let derived_authority = match &input.ingress {
            RuntimeAdmissionIngress::TrustedLocalSystem => {
                if caller_ura != crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
                    return Err(Status::permission_denied(
                        "trusted local-system admission requires exact `_system.local` caller",
                    ));
                }
                let metadata_authority = verify_local_system_authority_metadata(
                    &input.envelope,
                    &input.ability,
                    descriptor.action,
                    Some(&input.metadata),
                    current_unix_ms(),
                )
                .map_err(|status| {
                    self.authority_denied_status(&input.envelope, &input.ability, status)
                })?;
                let carries_authority_proof = input
                    .metadata
                    .get(AUTHORITY_PROOF_METADATA_KEY)
                    .is_some_and(|value| !value.trim().is_empty());
                if let Some(authority) = metadata_authority {
                    if carries_authority_proof {
                        return Err(self.authority_denied_status(
                            &input.envelope,
                            &input.ability,
                            Status::invalid_argument(format!(
                                "{REASON_AUTHORITY_FORMAT_INVALID}: invocation carries multiple independent authority proofs"
                            )),
                        ));
                    }
                    return runtime_admission_decision(
                        admitted_envelope,
                        authority.with_runtime_admission_fact(
                            "trusted-local-system metadata authority admission",
                        )?,
                        RuntimeAdmissionReservation { quota: None },
                    );
                }
                if carries_authority_proof {
                    return Err(self.authority_denied_status(
                        &input.envelope,
                        &input.ability,
                        Status::invalid_argument(format!(
                            "{REASON_AUTHORITY_FORMAT_INVALID}: trusted local-system ingress cannot carry an unverified `{AUTHORITY_PROOF_METADATA_KEY}`"
                        )),
                    ));
                }
                return runtime_admission_decision(
                    admitted_envelope,
                    VerifiedRuntimeAuthority::trusted_local_system_capability(
                        admitted_envelope.envelope(),
                    )?
                    .with_runtime_admission_fact("trusted-local-system capability admission")?,
                    RuntimeAdmissionReservation { quota: None },
                );
            }
            RuntimeAdmissionIngress::BootstrapCandidate => {
                Self::verify_bootstrap_candidate(
                    &input.envelope,
                    &input.ability,
                    &input.arguments,
                )?;
                reject_public_hosted_agent_delegation_metadata(Some(&input.metadata))?;
                reject_unverified_runtime_authority_metadata(
                    Some(&input.metadata),
                    "bootstrap-candidate admission",
                )?;
                if descriptor.action != AccessAction::Manage {
                    return Err(self.authority_denied_status(
                        &input.envelope,
                        &input.ability,
                        Status::permission_denied(format!(
                            "{REASON_AUTHORITY_REQUIRED}: bootstrap ability `{}` must declare manage admission action",
                            input.ability
                        )),
                    ));
                }
                return runtime_admission_decision(
                    admitted_envelope,
                    VerifiedRuntimeAuthority::bootstrap(admitted_envelope.envelope(), None)?
                        .with_runtime_admission_fact("bootstrap-candidate admission")?,
                    RuntimeAdmissionReservation { quota: None },
                );
            }
            RuntimeAdmissionIngress::CallerSigned => None,
            RuntimeAdmissionIngress::DerivedChild(authority) => Some(authority),
        };

        let trust_anchor = self.trust_anchor.snapshot();
        let device_purpose = if parse_ura(caller_ura)
            .map(|caller| caller.kind == URAKind::Device)
            .unwrap_or(false)
        {
            Some(
                verify_device_invocation_purpose(DeviceInvocationPurposeScope {
                    caller_ura,
                    callee_ura: callee_ura_required(&input.envelope)?,
                    subject_ura: subject_ura_required(&input.envelope)?,
                    public_ability: &input.ability,
                    daemon_ura: self.daemon_ura.as_deref(),
                    action: descriptor.action,
                })
                .map_err(|error| {
                    self.signature_denied_status(
                        &input.envelope,
                        &input.ability,
                        Status::permission_denied(format!(
                            "DEVICE_CALLER_PURPOSE_DENIED: {error:?}"
                        )),
                    )
                })?,
            )
        } else {
            None
        };
        let trusted_path = if derived_authority.is_some() {
            TrustedCallerPath::from_derived_child_caller(caller_ura)
        } else {
            self.trusted_path_for_caller(
                caller_ura,
                trust_anchor.as_ref(),
                &input.ability,
                device_purpose,
            )
        }
        .map_err(|status| self.signature_denied_status(&input.envelope, &input.ability, status))?;
        reject_public_hosted_agent_delegation_metadata(Some(&input.metadata))?;
        let authority = self.enforce_runtime_admitted_policy(
            &input.envelope,
            &input.ability,
            &input.arguments,
            Some(&input.metadata),
            trust_anchor,
            trusted_path,
            descriptor.action,
            descriptor.safe_read,
            &descriptor_bound,
            derived_authority,
        )?;

        if input.call_mode != AxonCallMode::Rpc
            || !crate::daemon::invocation::admission::quota_meter::quota_meters_function(
                &input.ability,
            )
        {
            return runtime_admission_decision(
                admitted_envelope,
                authority,
                RuntimeAdmissionReservation { quota: None },
            );
        }
        let Some(quota) = self
            .quota
            .reserve(caller_ura, &input.ability, current_unix_ms())
        else {
            return runtime_admission_decision(
                admitted_envelope,
                authority,
                RuntimeAdmissionReservation { quota: None },
            );
        };
        let decision = quota.decision();
        if !decision.allowed {
            let status = quota_denied_status(caller_ura, &input.ability, decision);
            drop(quota);
            return Err(status);
        }
        runtime_admission_decision(
            admitted_envelope,
            authority,
            RuntimeAdmissionReservation { quota: Some(quota) },
        )
    }

    /// Attach the exact federated key provider already installed in Axon's
    /// canonical admission graph. The facade may classify policy context and
    /// serve resolve-key projections through it, but cannot replace it.
    #[must_use]
    pub fn with_federated_key_resolver(mut self, resolver: Arc<FederatedKeyResolver>) -> Self {
        self.federated_keys = Some(resolver);
        self
    }

    #[must_use]
    pub(crate) fn with_invocation_verification_keys(
        mut self,
        provider: Arc<
            dyn crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider,
        >,
    ) -> Self {
        self.invocation_verification_keys = Some(provider);
        self
    }

    fn trusted_path_for_caller(
        &self,
        caller_ura: &str,
        trust_anchor: &RealmTrustAnchor,
        public_ability: &str,
        device_purpose: Option<VerifiedDeviceInvocationPurpose>,
    ) -> Result<TrustedCallerPath, Status> {
        if let Some(entry) = trust_anchor.lookup(caller_ura) {
            return TrustedCallerPath::from_verified_invocation_caller(
                caller_ura,
                VerifiedCallerEvidence::TrustAnchorRole(entry.role),
                public_ability,
                device_purpose,
            );
        }
        if self.trust_anchor_user_role_for_caller(caller_ura, trust_anchor) {
            return Ok(TrustedCallerPath::User);
        }
        if let Some(role) = self.principal_lifecycle_role_for_caller(caller_ura)? {
            return TrustedCallerPath::from_verified_invocation_caller(
                caller_ura,
                VerifiedCallerEvidence::PrincipalLifecycleRole(role),
                public_ability,
                device_purpose,
            );
        }
        if let Some(provider) = self.invocation_verification_keys.as_ref() {
            let hosted = provider
                .resolve_invocation_verifying_key(caller_ura)
                .map_err(|error| {
                    Status::internal(format!(
                        "resolve local invocation verification key for {caller_ura}: {error}"
                    ))
                })?
                .is_some();
            if hosted {
                return TrustedCallerPath::from_verified_invocation_caller(
                    caller_ura,
                    VerifiedCallerEvidence::LocalHostedAgentKey,
                    public_ability,
                    device_purpose,
                );
            }
        }
        if self.has_hub_attested_caller(caller_ura) || self.is_federated_caller(caller_ura) {
            return TrustedCallerPath::from_verified_invocation_caller(
                caller_ura,
                VerifiedCallerEvidence::Federated,
                public_ability,
                device_purpose,
            );
        }
        Err(permission_denied_unknown_caller(caller_ura))
    }

    fn trust_anchor_user_role_for_caller(
        &self,
        caller_ura: &str,
        trust_anchor: &RealmTrustAnchor,
    ) -> bool {
        let Ok(caller) = parse_ura(caller_ura) else {
            return false;
        };
        if caller.kind != URAKind::User {
            return false;
        }
        let Some(daemon_realm) = self
            .daemon_ura
            .as_deref()
            .and_then(|daemon_ura| parse_ura(daemon_ura).ok())
            .map(|daemon| daemon.realm)
        else {
            return false;
        };
        caller.realm == daemon_realm && !trust_anchor.lookup_user_all(caller_ura).is_empty()
    }

    fn principal_lifecycle_role_for_caller(
        &self,
        caller_ura: &str,
    ) -> Result<Option<TrustAnchorRole>, Status> {
        let Some(reader) = self.principal_lifecycle.as_ref() else {
            return Ok(None);
        };
        let Ok(caller) = parse_ura(caller_ura) else {
            return Ok(None);
        };
        if caller.kind != URAKind::User {
            return Ok(None);
        }
        let Some(daemon_realm) = self
            .daemon_ura
            .as_deref()
            .and_then(|daemon_ura| parse_ura(daemon_ura).ok())
            .map(|daemon| daemon.realm)
        else {
            return Ok(None);
        };
        if caller.realm != daemon_realm {
            return Ok(None);
        }
        let state = reader.admission_state(caller_ura)?;
        match state {
            PrincipalAdmissionState::Active => Ok(Some(TrustAnchorRole::User)),
            PrincipalAdmissionState::Missing => Ok(None),
            PrincipalAdmissionState::Pending
            | PrincipalAdmissionState::Suspended
            | PrincipalAdmissionState::Deleted => Err(Status::permission_denied(format!(
                "PRINCIPAL_LIFECYCLE_DENIED: caller URA `{caller_ura}` is {} and cannot invoke",
                principal_admission_state_label(state)
            ))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn enforce_runtime_admitted_policy(
        &self,
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
        metadata: Option<&HashMap<String, String>>,
        trust_anchor: Arc<RealmTrustAnchor>,
        trusted_path: TrustedCallerPath,
        action: AccessAction,
        safe_read: bool,
        descriptor_bound: &WireDescriptorBoundEnvelope,
        derived_authority: Option<&DerivedRuntimeAuthorityContext>,
    ) -> Result<VerifiedRuntimeAuthority, Status> {
        self.enforce_principal_lifecycle_admission(envelope, ability, trusted_path)
            .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
        if uses_bootstrap_authority(envelope, ability) {
            reject_unverified_runtime_authority_metadata(metadata, "bootstrap-authority admission")
                .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
            return VerifiedRuntimeAuthority::bootstrap(
                descriptor_bound.envelope.envelope(),
                None,
            )?
            .with_runtime_admission_fact("bootstrap-authority admission");
        }
        // `federation.advertise_agent` has a stronger typed authority proof:
        // the signed Device caller, Authority callee, exact Agent subject, and
        // durable Device->User owner binding are verified as one publication
        // tuple. Establish that fact before the generic subject-authority gate
        // so Device custody is not mis-modelled as delegation authority over
        // an Agent. Any independent authority carrier remains ambiguous and is
        // rejected below instead of being merged with this proof.
        let hosted_agent_publication_authority_id = self
            .verify_hosted_agent_publication_authority(
                envelope,
                ability,
                args,
                trust_anchor.as_ref(),
            )
            .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
        let peer_directory_stream_authority = {
            let caller_ura = caller_ura_required(envelope)?;
            let callee_ura = callee_ura_required(envelope)?;
            let subject_ura = subject_ura_required(envelope)?;
            let ability_ura = ability_ura_for(callee_ura, ability)?;
            VerifiedAuthorityPeerDirectoryStream::classify(
                caller_ura,
                callee_ura,
                subject_ura,
                &ability_ura,
                self.daemon_ura.as_deref(),
                trusted_path,
                action,
                trust_anchor.as_ref(),
            )
            .into_result()
            .map_err(|reason| {
                self.authority_denied_status(
                    envelope,
                    ability,
                    Status::permission_denied(format!(
                        "PEER_DIRECTORY_STREAM_AUTHORITY_DENIED: {reason}"
                    )),
                )
            })?
        };
        let carries_authority_proof = metadata
            .and_then(|values| values.get(AUTHORITY_PROOF_METADATA_KEY))
            .is_some_and(|value| !value.trim().is_empty());
        let typed_authority_ingress = if hosted_agent_publication_authority_id.is_some() {
            Some("hosted-agent-publication admission")
        } else if peer_directory_stream_authority.is_some() {
            Some("peer-directory-stream admission")
        } else if derived_authority.is_some() {
            Some("runtime-derived-child admission")
        } else {
            None
        };
        if let Some(ingress) = typed_authority_ingress {
            reject_independent_authority_carriers(metadata, ingress)
                .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
        }
        let metadata_authority = if typed_authority_ingress.is_some() {
            None
        } else {
            verify_delegation_metadata(
                envelope,
                ability,
                action,
                metadata,
                trust_anchor.as_ref(),
                self.federated_keys.as_deref(),
                current_unix_ms(),
            )
            .map_err(|status| self.authority_denied_status(envelope, ability, status))?
        };
        if metadata_authority.is_some() && carries_authority_proof {
            return Err(self.authority_denied_status(
                envelope,
                ability,
                Status::invalid_argument(format!(
                    "{REASON_AUTHORITY_FORMAT_INVALID}: invocation carries multiple independent authority proofs"
                )),
            ));
        }
        let authority_proof_authority = if typed_authority_ingress.is_some() {
            None
        } else {
            self.verify_authority_proof_metadata(AuthorityProofMetadataInput {
                envelope,
                ability,
                action,
                metadata,
                trust_anchor: trust_anchor.as_ref(),
                trusted_path,
                descriptor_bound,
            })
            .map_err(|status| self.authority_denied_status(envelope, ability, status))?
        };
        let bootstrap_authority_id = match BootstrapAuthorityVerifier::verify(
            envelope,
            ability,
            action,
            args,
            trust_anchor.as_ref(),
            trusted_path,
            self.daemon_ura.as_deref(),
        ) {
            BootstrapAuthorityDecision::Verified { authority_id } => Some(authority_id),
            BootstrapAuthorityDecision::Unavailable { message } => {
                return Err(self.authority_denied_status(
                    envelope,
                    ability,
                    Status::failed_precondition(message),
                ));
            }
            BootstrapAuthorityDecision::NotApplicable => None,
        };
        let verified_authority_id = authority_proof_authority
            .as_ref()
            .and_then(VerifiedRuntimeAuthority::authority_id)
            .or_else(|| {
                metadata_authority
                    .as_ref()
                    .and_then(VerifiedRuntimeAuthority::authority_id)
            })
            .or(bootstrap_authority_id.as_deref())
            .or(hosted_agent_publication_authority_id.as_deref())
            .or_else(|| {
                peer_directory_stream_authority
                    .as_ref()
                    .map(VerifiedAuthorityPeerDirectoryStream::authority_id)
            })
            .map(ToOwned::to_owned);
        let verified_session_id = authority_proof_authority
            .as_ref()
            .and_then(VerifiedRuntimeAuthority::session_id)
            .or_else(|| {
                metadata_authority
                    .as_ref()
                    .and_then(VerifiedRuntimeAuthority::session_id)
            })
            .map(ToOwned::to_owned);
        let policy_decision = AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope,
            ability,
            action,
            safe_read,
            trusted_path,
            daemon_ura: self.daemon_ura.as_deref(),
            trust_anchor: trust_anchor.as_ref(),
            access_control_stores: self.access_control_stores.as_ref(),
            canonical_hash: Some(format!(
                "sha256:{}",
                hex::encode(sha2::Sha256::digest(descriptor_bound_canonical_bytes(
                    &descriptor_bound.envelope
                )))
            )),
            signature_key_id: envelope
                .caller_signature
                .as_ref()
                .map(|signature| signature.key_id_hint.clone())
                .filter(|key| !key.is_empty()),
            verified_authority_id,
            verified_session_id,
            accountable_principal: derived_authority
                .and_then(DerivedRuntimeAuthorityContext::accountable_user_ura)
                .map(PrincipalProjection::accountable_user)
                .transpose()?,
            rejector_ura: self.daemon_ura.clone(),
        })?;
        if let Some(authority) = authority_proof_authority {
            return authority.with_policy_decision(&policy_decision);
        }
        if let Some(authority) = metadata_authority {
            return authority.with_policy_decision(&policy_decision);
        }
        if let Some(authority) = peer_directory_stream_authority {
            if policy_decision.policy_rule_id.as_deref()
                != Some("system.authority.peer_directory_stream")
            {
                return Err(Status::internal(
                    "peer directory authority fact was not confirmed by the exact system policy rule",
                ));
            }
            return VerifiedRuntimeAuthority::policy(
                descriptor_bound.envelope.envelope(),
                authority.policy_ura(),
                Some(authority.authority_id().to_string()),
            )?
            .with_policy_decision(&policy_decision);
        }
        if let Some(authority_id) = bootstrap_authority_id.or(hosted_agent_publication_authority_id)
        {
            return VerifiedRuntimeAuthority::bootstrap(
                descriptor_bound.envelope.envelope(),
                Some(authority_id),
            )?
            .with_policy_decision(&policy_decision);
        }
        if let Some(derived_authority) = derived_authority {
            return VerifiedRuntimeAuthority::capability(
                descriptor_bound.envelope.envelope(),
                &derived_authority.capability_ura,
            )?
            .with_policy_decision(&policy_decision);
        }
        VerifiedRuntimeAuthority::self_authority(
            descriptor_bound.envelope.envelope().caller.ura.clone(),
        )
        .with_policy_decision(&policy_decision)
    }

    fn verify_hosted_agent_publication_authority(
        &self,
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
        trust_anchor: &RealmTrustAnchor,
    ) -> Result<Option<String>, Status> {
        if ability != ABILITY_FEDERATION_ADVERTISE_AGENT {
            return Ok(None);
        }
        let request: AdvertiseAgentRequest = serde_json::from_slice(args).map_err(|err| {
            Status::invalid_argument(format!(
                "federation.advertise_agent: arguments JSON decode failed: {err}"
            ))
        })?;
        let publication = HostedAgentPublication::verify(
            envelope,
            &request,
            trust_anchor,
            self.daemon_ura.as_deref(),
        )
        .map_err(|err| {
            Status::permission_denied(format!(
                "federation.advertise_agent: hosted publication authority denied: {err}"
            ))
        })?;
        Ok(Some(publication.authority_id()))
    }

    fn enforce_principal_lifecycle_admission(
        &self,
        envelope: &Envelope,
        ability: &str,
        trusted_path: TrustedCallerPath,
    ) -> Result<(), Status> {
        if trusted_path != TrustedCallerPath::User {
            return Ok(());
        }
        let Some(reader) = self.principal_lifecycle.as_ref() else {
            return Ok(());
        };
        let principal_ura = caller_ura_required(envelope)?;
        let state = reader.admission_state(principal_ura)?;
        match state {
            PrincipalAdmissionState::Missing | PrincipalAdmissionState::Active => Ok(()),
            PrincipalAdmissionState::Pending
            | PrincipalAdmissionState::Suspended
            | PrincipalAdmissionState::Deleted => Err(Status::permission_denied(format!(
                "PRINCIPAL_LIFECYCLE_DENIED: principal_ura `{principal_ura}` is {} and cannot invoke `{ability}`",
                principal_admission_state_label(state)
            ))),
        }
    }

    fn verify_bootstrap_candidate(
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
    ) -> Result<(), Status> {
        match ability {
            ABILITY_FEDERATION_JOIN => Self::verify_bootstrap_federation_join(envelope, args),
            ABILITY_IDENTITY_REGISTER_PUBKEY => {
                Self::verify_bootstrap_identity_register_pubkey(envelope, args)
            }
            _ => Err(permission_denied_unknown_caller(
                envelope
                    .caller
                    .as_ref()
                    .map(|caller| caller.ura.as_str())
                    .unwrap_or("<missing>"),
            )),
        }
    }

    fn verify_bootstrap_federation_join(envelope: &Envelope, args: &[u8]) -> Result<(), Status> {
        let caller = caller_ura_required(envelope)?;
        let callee_ura = envelope
            .callee
            .as_ref()
            .map(|callee| callee.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| Status::invalid_argument("federation.join missing hub callee"))?;
        let subject_ura = envelope
            .subject
            .as_ref()
            .map(|subject| subject.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument("federation.join missing membership subject")
            })?;
        let callee = crate::core::ura::parse_ura(callee_ura).map_err(|err| {
            Status::invalid_argument(format!("federation.join callee is not a hub URA: {err}"))
        })?;
        if callee.kind != crate::core::ura::URAKind::Authority {
            return Err(Status::invalid_argument(format!(
                "federation.join callee must identify a hub, got {:?}",
                callee.kind
            )));
        }
        let caller_parsed = crate::core::ura::parse_ura(caller).map_err(|err| {
            Status::invalid_argument(format!("federation.join caller is not a device URA: {err}"))
        })?;
        if caller_parsed.kind != crate::core::ura::URAKind::Device {
            return Err(Status::invalid_argument(format!(
                "federation.join caller must identify a device, got {:?}",
                caller_parsed.kind
            )));
        }
        let subject = crate::core::ura::parse_ura(subject_ura).map_err(|err| {
            Status::invalid_argument(format!(
                "federation.join subject is not a device URA: {err}"
            ))
        })?;
        if subject.kind != crate::core::ura::URAKind::Device {
            return Err(Status::invalid_argument(format!(
                "federation.join subject must identify a device, got {:?}",
                subject.kind
            )));
        }
        let request: crate::daemon::invocation::dispatch::federation_wrappers::JoinRequest =
            serde_json::from_slice(args).map_err(|err| {
                Status::invalid_argument(format!("federation.join args JSON decode failed: {err}"))
            })?;
        if request.realm != callee.realm
            || request.realm != caller_parsed.realm
            || request.realm != subject.realm
        {
            return Err(Status::invalid_argument(format!(
                "federation.join realm mismatch: request={}, callee={}, caller={}, subject={}",
                request.realm, callee.realm, caller_parsed.realm, subject.realm
            )));
        }
        if request.membership_ura != caller {
            return Err(Status::invalid_argument(
                "federation.join membership_ura must match envelope caller",
            ));
        }
        if request.membership_ura != subject_ura {
            return Err(Status::invalid_argument(
                "federation.join membership_ura must match envelope subject",
            ));
        }
        let public_key = hex::decode(request.public_key_hex.trim()).map_err(|err| {
            Status::invalid_argument(format!("federation.join public_key_hex is not hex: {err}"))
        })?;
        if public_key.len() != 32 {
            return Err(Status::invalid_argument(format!(
                "federation.join public_key_hex must decode to 32 bytes, got {}",
                public_key.len()
            )));
        }
        Ok(())
    }

    fn verify_bootstrap_identity_register_pubkey(
        envelope: &Envelope,
        args: &[u8],
    ) -> Result<(), Status> {
        verify_user_register_pubkey_bootstrap_claim(envelope, args).map(|_| ())
    }

    fn accepts_local_self_caller(&self, caller_ura: &str) -> bool {
        // Off-box transports never get local self admission, even on an exact
        // daemon-URA match: the same URA an attacker can put in `caller.ura`
        // would otherwise skip the entire strict pipeline.
        self.transport_boundary
            .accepts_local_self_caller(self.daemon_ura.as_deref(), caller_ura)
    }

    /// Classify the one ingress that may be reissued by
    /// `SystemInvocationIssuer`.
    ///
    /// A daemon/device URA accepted by the local transport policy is still a
    /// real caller identity and must retain its supplied signature at the
    /// canonical runtime boundary. Only the reserved `_system.local` caller on
    /// a local-only transport is a system-issued invocation.
    pub(crate) fn accepts_local_system_envelope(&self, envelope: Option<&Envelope>) -> bool {
        let Some(caller_ura) = envelope
            .and_then(|envelope| envelope.caller.as_ref())
            .map(|caller| caller.ura.trim())
            .filter(|caller| !caller.is_empty())
        else {
            return false;
        };
        caller_ura == crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
            && self.accepts_local_self_caller(caller_ura)
    }

    /// Classify callers against the same live peer map used by canonical
    /// caller-key resolution.
    fn is_federated_caller(&self, caller_ura: &str) -> bool {
        self.federated_keys
            .as_ref()
            .is_some_and(|resolver| resolver.is_configured_federated_caller(caller_ura))
    }

    /// Consume the same exact, expiring Hub attestation already used by the
    /// canonical signature verifier. Device runtimes intentionally have no
    /// peer federation client, so their policy path must not depend on the
    /// Hub-only peer directory after upstream key synchronization succeeds.
    fn has_hub_attested_caller(&self, caller_ura: &str) -> bool {
        self.federated_keys
            .as_ref()
            .is_some_and(|resolver| resolver.has_hub_attested_caller(caller_ura))
    }

    fn signature_denied_status(
        &self,
        envelope: &Envelope,
        ability: &str,
        status: Status,
    ) -> Status {
        admission_denied_status(
            status,
            "SIGNATURE_DENIED",
            "signature",
            signature_reason_from_status,
            envelope,
            ability,
            self.daemon_ura.as_deref(),
        )
    }

    fn authority_denied_status(
        &self,
        envelope: &Envelope,
        ability: &str,
        status: Status,
    ) -> Status {
        admission_denied_status(
            status,
            "AUTHORITY_DENIED",
            "authority",
            authority_reason_from_status,
            envelope,
            ability,
            self.daemon_ura.as_deref(),
        )
    }

    fn verify_authority_proof_metadata(
        &self,
        input: AuthorityProofMetadataInput<'_>,
    ) -> Result<Option<VerifiedRuntimeAuthority>, Status> {
        let AuthorityProofMetadataInput {
            envelope,
            ability,
            action,
            metadata,
            trust_anchor,
            trusted_path,
            descriptor_bound,
        } = input;
        let Some(proof) = authority_proof_from_metadata(metadata)? else {
            return Ok(None);
        };
        let caller_ura = caller_ura_required(envelope)?;
        let callee_ura = callee_ura_required(envelope)?;
        let subject_ura = subject_ura_required(envelope)?;
        let ability_ura = ability_ura_for(callee_ura, ability)?;
        let principal = principal_for(trusted_path, caller_ura, trust_anchor)?;
        let canonical_hash = format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(descriptor_bound_canonical_bytes(
                &descriptor_bound.envelope
            )))
        );
        let invocation_nonce = invocation_nonce_for_proof(envelope);
        let audience_ura = self.authority_proof_audience_ura()?;
        let now = Utc::now();
        self.access_control_stores
            .with_store(&proof.owner_user_ura, |store| {
                let resolver = StoreBackedAuthorityProofResolver {
                    trust_anchor,
                    store,
                    now,
                };
                let context = AuthorityProofVerificationContext {
                    owner_user_ura: &proof.owner_user_ura,
                    principal_kind: principal.kind,
                    principal_id: &principal.id,
                    token_id: principal.token_id.as_deref(),
                    token_class: principal.token_class,
                    callee_ura,
                    subject_ura,
                    ability_ura: &ability_ura,
                    action,
                    nonce: invocation_nonce.as_deref(),
                    canonical_hash: Some(canonical_hash.as_str()),
                    audience_ura,
                    session_id: proof.session_id.as_deref(),
                    session_owner_user_ura: proof.session_owner_user_ura.as_deref(),
                    now,
                };
                AuthorityProofVerifier::verify(Some(&proof), &context, &resolver)
                    .map_err(authority_proof_status)?;
                if request_scoped_one_time_authority_proof(&proof) {
                    store
                        .consume_authority_proof_once(&proof.proof_id, audience_ura)
                        .map_err(|err| {
                            Status::permission_denied(format!(
                                "{}: {err}",
                                AuthorityProofDenyReason::AuthorityProofRevoked.as_str()
                            ))
                        })?;
                }
                Ok::<(), Status>(())
            })
            .map_err(|err| {
                Status::internal(format!(
                    "POLICY_STORE_UNAVAILABLE: owner_user_id={} error={err}",
                    proof.owner_user_ura
                ))
            })??;
        VerifiedRuntimeAuthority::from_authority_proof(&proof).map(Some)
    }
}

/// Daemon-owned capability for applying the exact same runtime-admission
/// transaction to a runtime-derived child as to a carrier-delivered request.
///
/// Axon owns child derivation, signatures, replay, lifecycle, and receipts.
/// This object contributes only the downstream runtime admission and quota
/// context that cannot be encoded in the canonical descriptor-bound envelope.
#[derive(Clone)]
pub(crate) struct DaemonDerivedInvocationAdmission {
    facade: AdmissionFacade,
    coordinator: Arc<DaemonRuntimeAdmissionCoordinator>,
}

impl DaemonDerivedInvocationAdmission {
    pub(crate) fn new(
        facade: AdmissionFacade,
        coordinator: Arc<DaemonRuntimeAdmissionCoordinator>,
    ) -> Self {
        Self {
            facade,
            coordinator,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the admission seam preserves the complete signed child tuple at the provider boundary"
    )]
    pub(crate) fn stage(
        &self,
        parent: &AbilityContext,
        descriptor_bound: &DescriptorBoundEnvelope,
        signed_envelope: &axon_sdk::invocation::SignedEnvelope,
        arguments: Vec<u8>,
        metadata: HashMap<String, String>,
        request_id: String,
        ability: &str,
        call_mode: AxonCallMode,
    ) -> Result<DaemonRuntimeAdmissionLease, Status> {
        self.coordinator.stage_derived(
            &self.facade,
            parent,
            descriptor_bound,
            signed_envelope,
            arguments,
            metadata,
            request_id,
            ability,
            call_mode,
        )
    }
}

fn default_access_control_stores() -> Arc<AccessControlStoreRegistry> {
    #[cfg(test)]
    {
        Arc::new(AccessControlStoreRegistry::ephemeral())
    }
    #[cfg(not(test))]
    {
        Arc::new(AccessControlStoreRegistry::default())
    }
}

fn runtime_admission_decision(
    envelope: &DescriptorBoundEnvelope,
    authority: VerifiedRuntimeAuthority,
    reservation: RuntimeAdmissionReservation,
) -> Result<RuntimeAdmissionDecision, Status> {
    let policy = authority
        .into_policy(envelope)
        .map_err(axon_error_to_status)?;
    Ok(RuntimeAdmissionDecision {
        reservation,
        policy,
    })
}

fn runtime_admission_envelope(
    admitted: &axon_sdk::invocation::InvocationEnvelope,
    caller_signature: Option<axon_sdk::invocation::CallerSignature>,
    request_id: String,
) -> Result<Envelope, Status> {
    axon_sdk::invocation::project_wire_envelope(
        admitted,
        axon_sdk::invocation::WireEnvelopeMetadata {
            request_id,
            caller_signature,
            ..axon_sdk::invocation::WireEnvelopeMetadata::default()
        },
    )
    .map_err(|error| {
        Status::internal(format!(
            "project admitted canonical envelope for runtime admission: {error}"
        ))
    })
}

fn wire_caller_signature(
    wire: &crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatch,
) -> Result<axon_sdk::invocation::CallerSignature, Status> {
    match &wire.ingress {
        crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::ExternalSigned(
            signature,
        )
        | crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::BootstrapCandidate(
            signature,
        ) => Ok(signature.clone()),
        crate::daemon::axon_bridge::descriptor_bound_dispatch::WireDispatchIngress::LocalSystem => Err(
            Status::internal("trusted local-system ingress has no caller signature"),
        ),
    }
}

fn daemon_call_mode(call_mode: AxonCallMode) -> crate::daemon::ability::CallMode {
    match call_mode {
        AxonCallMode::Rpc => crate::daemon::ability::CallMode::Rpc,
        AxonCallMode::Stream => crate::daemon::ability::CallMode::Stream,
        AxonCallMode::Bidi => crate::daemon::ability::CallMode::Bidi,
    }
}

fn quota_denied_status(
    caller_ura: &str,
    ability: &str,
    decision: crate::daemon::invocation::admission::usage_quota::QuotaDecision,
) -> Status {
    match decision.deny_reason {
        Some(QuotaDenyReason::KeyTooLarge) => Status::invalid_argument(format!(
            "REQUEST_METADATA_INVALID: quota key too large caller={caller_ura} ability={ability}"
        )),
        Some(QuotaDenyReason::StoreSaturated) => Status::resource_exhausted(format!(
            "RESOURCE_EXHAUSTED: quota store saturated caller={caller_ura} ability={ability} retry_after_ms={}",
            decision.retry_after_ms
        )),
        Some(QuotaDenyReason::BudgetExhausted) | None => Status::resource_exhausted(format!(
            "QUOTA_EXCEEDED: caller={caller_ura} ability={ability} retry_after_ms={}",
            decision.retry_after_ms
        )),
    }
}

fn runtime_admission_status_to_axon(status: Status) -> InvocationError {
    let detail = status.message().to_string();
    match status.code() {
        Code::Cancelled => InvocationError::cancelled(detail.clone())
            .with_code(ErrorCode::ExecutionFailed)
            .with_stage(ErrorStage::Execution)
            .with_security_class(SecurityClass::Internal)
            .with_message(detail),
        Code::DeadlineExceeded => InvocationError::deadline_exceeded(detail.clone())
            .with_code(ErrorCode::ExecutionFailed)
            .with_stage(ErrorStage::Execution)
            .with_security_class(SecurityClass::Internal)
            .with_message(detail),
        Code::InvalidArgument | Code::OutOfRange => classify_runtime_admission_invalid(detail),
        Code::ResourceExhausted => classify_runtime_admission_quota(detail),
        Code::Unavailable => InvocationError::unavailable(detail.clone())
            .with_code(ErrorCode::TransportUntrusted)
            .with_stage(ErrorStage::Transport)
            .with_security_class(SecurityClass::Transport)
            .with_message(detail),
        Code::PermissionDenied | Code::Unauthenticated => {
            classify_runtime_admission_permission_denied(detail)
        }
        Code::Ok
        | Code::Unknown
        | Code::NotFound
        | Code::AlreadyExists
        | Code::FailedPrecondition
        | Code::Aborted
        | Code::Unimplemented
        | Code::Internal
        | Code::DataLoss => InvocationError::internal(detail.clone())
            .with_code(ErrorCode::InternalError)
            .with_stage(ErrorStage::GlobalAdmission)
            .with_security_class(SecurityClass::Internal)
            .with_message(detail),
    }
}

fn classify_runtime_admission_invalid(detail: String) -> InvocationError {
    let (code, stage, security_class) = if detail.contains("REQUEST_METADATA_INVALID") {
        (
            ErrorCode::RequestMetadataInvalid,
            ErrorStage::RequestValidation,
            SecurityClass::Resource,
        )
    } else if detail.contains(REASON_ENVELOPE_INCOMPLETE) {
        (
            ErrorCode::RequestPayloadInvalid,
            ErrorStage::GlobalAdmission,
            SecurityClass::Identity,
        )
    } else if detail.contains(REASON_CALLER_SIGNATURE_INVALID) {
        (
            ErrorCode::CallerSignatureInvalid,
            ErrorStage::CallerAuthentication,
            SecurityClass::Authentication,
        )
    } else if detail.contains(REASON_NONCE_REPLAY) {
        (
            ErrorCode::CallerNonceReplayed,
            ErrorStage::CallerAuthentication,
            SecurityClass::Authentication,
        )
    } else {
        (
            ErrorCode::RequestPayloadInvalid,
            ErrorStage::RequestValidation,
            SecurityClass::Internal,
        )
    };
    InvocationError::invalid_argument(detail.clone())
        .with_code(code)
        .with_stage(stage)
        .with_security_class(security_class)
        .with_message(detail)
}

fn classify_runtime_admission_quota(detail: String) -> InvocationError {
    let code = if detail.contains("RESOURCE_EXHAUSTED") {
        ErrorCode::ResourceExhausted
    } else {
        ErrorCode::QuotaExceeded
    };
    InvocationError::resource_exhausted(detail.clone())
        .with_code(code)
        .with_stage(ErrorStage::Quota)
        .with_security_class(SecurityClass::Resource)
        .with_message(detail)
}

fn classify_runtime_admission_permission_denied(detail: String) -> InvocationError {
    let (code, stage, security_class) = if detail.contains("HOSTED_AGENT_NOT_PUBLISHED") {
        (
            ErrorCode::AbilityDisabled,
            ErrorStage::AbilityPolicy,
            SecurityClass::Authorization,
        )
    } else if detail.contains("AUTHORITY_") {
        (
            authority_error_code_for_detail(&detail),
            ErrorStage::AuthorityValidation,
            SecurityClass::Authority,
        )
    } else if detail.contains("POLICY_DENIED") {
        (
            ErrorCode::AbilityForbidden,
            ErrorStage::AbilityPolicy,
            SecurityClass::Authorization,
        )
    } else if detail.contains("BOOTSTRAP") || detail.contains("bootstrap") {
        (
            ErrorCode::BootstrapNotAllowed,
            ErrorStage::BootstrapAuthorization,
            SecurityClass::Bootstrap,
        )
    } else {
        (
            ErrorCode::AbilityForbidden,
            ErrorStage::AbilityPolicy,
            SecurityClass::Authorization,
        )
    };
    InvocationError::permission_denied(detail.clone())
        .with_code(code)
        .with_stage(stage)
        .with_security_class(security_class)
        .with_message(detail)
}

fn authority_error_code_for_detail(detail: &str) -> ErrorCode {
    if detail.contains("AUTHORITY_SUBJECT_MISMATCH") {
        ErrorCode::AuthoritySubjectMismatch
    } else if detail.contains("AUTHORITY_CALLER_MISMATCH") {
        ErrorCode::AuthorityCallerMismatch
    } else if detail.contains("AUTHORITY_AUDIENCE_VIOLATION") {
        ErrorCode::AuthorityAudienceViolation
    } else if detail.contains("AUTHORITY_SCOPE_VIOLATION") {
        ErrorCode::AuthorityScopeViolation
    } else if detail.contains("AUTHORITY_REALM_MISMATCH") {
        ErrorCode::AuthorityRealmMismatch
    } else if detail.contains("AUTHORITY_EXPIRED") {
        ErrorCode::AuthorityExpired
    } else if detail.contains("AUTHORITY_REQUIRED") {
        ErrorCode::AuthorityRequired
    } else {
        ErrorCode::AuthorityChainInvalid
    }
}

struct StoreBackedAuthorityProofResolver<'a> {
    trust_anchor: &'a RealmTrustAnchor,
    store: &'a AccessControlStore,
    now: DateTime<Utc>,
}

impl AuthorityProofIssuerResolver for StoreBackedAuthorityProofResolver<'_> {
    fn verifying_key_for_issuer(&self, issuer_ura: &str) -> Option<VerifyingKey> {
        let parsed = crate::core::ura::parse_ura(issuer_ura).ok()?;
        let public_key_b64 = if parsed.kind == URAKind::User {
            let entries = self.trust_anchor.lookup_user_all(issuer_ura);
            let [entry] = entries else {
                return None;
            };
            entry.public_key_b64.as_str()
        } else {
            self.trust_anchor
                .lookup(issuer_ura)?
                .public_key_b64
                .as_str()
        };
        let bytes = BASE64_STANDARD.decode(public_key_b64.as_bytes()).ok()?;
        let bytes: [u8; ed25519_dalek::PUBLIC_KEY_LENGTH] = bytes.try_into().ok()?;
        VerifyingKey::from_bytes(&bytes).ok()
    }

    fn issuer_authorized_for_owner_ura(&self, issuer_ura: &str, owner_user_ura: &str) -> bool {
        if self.trust_anchor.lookup_user_all(issuer_ura).is_empty() {
            return false;
        }
        crate::core::ura::parse_ura(issuer_ura)
            .ok()
            .filter(|parsed| parsed.kind == URAKind::User)
            .and_then(|parsed| {
                parsed
                    .user_id()
                    .map(|user_id| crate::core::ura::user_ura(&parsed.realm, user_id))
            })
            .is_some_and(|issuer_user_ura| issuer_user_ura == owner_user_ura)
    }

    fn referenced_authority_active(&self, proof: &AuthorityProof) -> bool {
        if self.store.proof_consumed(&proof.proof_id) {
            return false;
        }
        if self
            .store
            .proof(&proof.proof_id)
            .is_none_or(|stored| stored.canonical_hash() != proof.canonical_hash())
        {
            return false;
        }
        match (
            proof.grant_id.as_deref(),
            proof.permission_request_id.as_deref(),
        ) {
            (Some(grant_id), None) => self
                .store
                .grant(grant_id)
                .is_some_and(|grant| grant_authorizes_proof(grant, proof, self.now)),
            (None, Some(request_id)) => self.store.request(request_id).is_some_and(|request| {
                request.status == PermissionRequestStatus::Approved
                    && request.authority_proof_id.as_deref() == Some(proof.proof_id.as_str())
            }),
            _ => false,
        }
    }
}

fn grant_authorizes_proof(
    grant: &PermissionGrant,
    proof: &AuthorityProof,
    now: DateTime<Utc>,
) -> bool {
    let matcher = PermissionGrantMatcher::new(std::slice::from_ref(grant));
    let input = GrantMatchInput {
        owner_user_ura: &proof.owner_user_ura,
        principal_kind: proof.principal_kind,
        principal_id: &proof.principal_id,
        token_id: proof.token_id.as_deref(),
        token_class: proof.token_class,
        session_id: proof.session_id.as_deref(),
        callee_ura: &proof.callee_ura,
        subject_ura: &proof.subject_ura,
        ability_ura: &proof.ability_ura,
        action: proof.action,
        now,
    };
    matcher
        .find_active(&input, PermissionEffect::Allow)
        .is_some_and(|matched| matched.grant_id == grant.grant_id)
}

fn authority_proof_from_metadata(
    metadata: Option<&HashMap<String, String>>,
) -> Result<Option<AuthorityProof>, Status> {
    let Some(raw) = metadata
        .and_then(|metadata| metadata.get(AUTHORITY_PROOF_METADATA_KEY))
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    serde_json::from_str::<AuthorityProof>(raw)
        .map(Some)
        .map_err(|err| {
            Status::invalid_argument(format!(
                "{}: `{AUTHORITY_PROOF_METADATA_KEY}` must contain an AuthorityProof JSON object: {err}",
                AuthorityProofDenyReason::AuthorityProofMismatch.as_str()
            ))
        })
}

fn authority_proof_status(reason: AuthorityProofDenyReason) -> Status {
    Status::permission_denied(reason.as_str().to_string())
}

fn invocation_nonce_for_proof(envelope: &Envelope) -> Option<String> {
    (!envelope.invocation_nonce.is_empty()).then(|| hex::encode(&envelope.invocation_nonce))
}

type AdmissionReasonExtractor = fn(&Status) -> String;

fn admission_denied_status(
    status: Status,
    outer_code: &'static str,
    target_stage: &'static str,
    reason_from_status: AdmissionReasonExtractor,
    envelope: &Envelope,
    ability: &str,
    rejector_ura: Option<&str>,
) -> Status {
    let grpc_code = status.code();
    let detail = status.message().to_string();
    let target_reason = reason_from_status(&status);
    let payload = serde_json::json!({
        "outer_code": outer_code,
        "target_stage": target_stage,
        "target_reason": target_reason,
        "caller_ura": identity_ura_for_diagnostic(envelope.caller.as_ref()),
        "callee_ura": identity_ura_for_diagnostic(envelope.callee.as_ref()),
        "ability_ura": ability_ura_for_diagnostic(envelope, ability),
        "subject_ura": subject_ura_for_diagnostic(envelope),
        "rejector_ura": rejector_ura,
        "detail": detail,
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        format!(
            "{{\"outer_code\":\"{outer_code}\",\"target_stage\":\"{target_stage}\",\"target_reason\":\"{target_reason}\",\"detail\":\"{detail}\"}}"
        )
    });
    status_with_code(grpc_code, format!("{outer_code}: {encoded}"))
}

fn signature_reason_from_status(status: &Status) -> String {
    SignatureDecisionReason::from_admission_detail(status.message())
        .as_str()
        .to_string()
}

fn authority_reason_from_status(status: &Status) -> String {
    status
        .message()
        .split_once(':')
        .map(|(reason, _)| reason)
        .unwrap_or_else(|| status.message())
        .trim()
        .to_ascii_uppercase()
}

fn status_with_code(code: Code, message: String) -> Status {
    match code {
        Code::Ok => Status::unknown(message),
        Code::Cancelled => Status::cancelled(message),
        Code::Unknown => Status::unknown(message),
        Code::InvalidArgument => Status::invalid_argument(message),
        Code::DeadlineExceeded => Status::deadline_exceeded(message),
        Code::NotFound => Status::not_found(message),
        Code::AlreadyExists => Status::already_exists(message),
        Code::PermissionDenied => Status::permission_denied(message),
        Code::ResourceExhausted => Status::resource_exhausted(message),
        Code::FailedPrecondition => Status::failed_precondition(message),
        Code::Aborted => Status::aborted(message),
        Code::OutOfRange => Status::out_of_range(message),
        Code::Unimplemented => Status::unimplemented(message),
        Code::Internal => Status::internal(message),
        Code::Unavailable => Status::unavailable(message),
        Code::DataLoss => Status::data_loss(message),
        Code::Unauthenticated => Status::unauthenticated(message),
    }
}

fn identity_ura_for_diagnostic(
    identity: Option<&axon_sdk::pb::axon::v1::AgentIdentity>,
) -> Option<String> {
    identity
        .map(|identity| identity.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToString::to_string)
}

fn subject_ura_for_diagnostic(envelope: &Envelope) -> Option<String> {
    envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|ura| !ura.is_empty())
        .map(ToString::to_string)
}

fn ability_ura_for_diagnostic(envelope: &Envelope, ability: &str) -> String {
    AuthorityAbilityView::from_envelope(envelope, ability)
        .map(|view| view.ability_ura)
        .unwrap_or_else(|_| ability.to_string())
}

// ── Helpers ─────────────────────────────────────────────────────────

fn require_local_hosted_agent_publication_ready(
    owner_ura: &str,
    exact_host_device_ura: Option<&str>,
) -> Result<(), Status> {
    let owner = parse_ura(owner_ura).map_err(|error| {
        Status::failed_precondition(format!("HOSTED_AGENT_PUBLICATION_OWNER_INVALID: {error}"))
    })?;
    if owner.kind != URAKind::Agent
        || owner.agent_ids().is_none()
        || owner.device_agent_ids().is_some()
    {
        return Ok(());
    }
    let exact_host_device_ura = exact_host_device_ura.ok_or_else(|| {
        Status::permission_denied(format!(
            "HOSTED_AGENT_NOT_PUBLISHED: Agent owner `{owner_ura}` is not an exact local hosted authority"
        ))
    })?;
    crate::daemon::persistence::hosted_agent_publications::require_published_for_host(
        owner_ura,
        exact_host_device_ura,
    )
    .map_err(|error| Status::permission_denied(format!("HOSTED_AGENT_NOT_PUBLISHED: {error:#}")))
}

/// Bootstrap authority abilities mutate identity or presence roots.
/// They still require the caller to pass strict admission above; this
/// gate only keeps trust-anchor bootstrap out of normal user-delegation
/// semantics so stale backend issuer keys cannot deadlock key repair.
fn uses_bootstrap_authority(envelope: &Envelope, ability: &str) -> bool {
    match ability {
        // First-key User self-registration is the only register-pubkey tuple
        // whose authority is proven by the presented bootstrap key itself.
        // A trusted realm Authority registering a Device key is a normal
        // caller-signed identity mutation and must verify its supplied
        // authority metadata instead of being reclassified as bootstrap.
        ABILITY_IDENTITY_REGISTER_PUBKEY => RegisterPubkeyBootstrapTuple::matches(envelope),
        ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
        | ABILITY_IDENTITY_LIST_USER_PUBKEYS
        | ABILITY_IDENTITY_REVOKE_USER_PUBKEY => true,
        _ => false,
    }
}

fn public_ability_name_from_route_for_owner(owner_ura: &str, ability: &str) -> String {
    let trimmed = ability.trim();
    let ability_ura = trimmed
        .split_once('@')
        .map(|(ability_ura, _)| ability_ura)
        .unwrap_or(trimmed);
    AbilitySelector::parse(ability_ura)
        .map(|selector| selector.public_name().to_string())
        .unwrap_or_else(|_| crate::core::ura::descriptor_public_ability_name(owner_ura, trimmed))
}

fn verify_delegation_metadata(
    envelope: &Envelope,
    ability: &str,
    action: AccessAction,
    metadata: Option<&HashMap<String, String>>,
    trust_anchor: &RealmTrustAnchor,
    federated_keys: Option<&FederatedKeyResolver>,
    now_ms: i64,
) -> Result<Option<VerifiedRuntimeAuthority>, Status> {
    verify_authority_metadata_with_issuer_key(
        envelope,
        ability,
        action,
        metadata,
        now_ms,
        RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
        &|issuer_ura| {
            if let Some(resolver) = federated_keys {
                return resolver
                    .resolve_all(issuer_ura)
                    .map(|keys| {
                        keys.into_iter()
                            .map(|key| BASE64_STANDARD.encode(key.to_bytes()))
                            .collect()
                    })
                    .map_err(|error| {
                        Status::permission_denied(format!(
                            "{REASON_AUTHORITY_ISSUER_UNKNOWN}: authority issuer `{issuer_ura}` has no canonical invocation key: {error}"
                        ))
                    });
            }
            let user_keys = trust_anchor.lookup_user_all(issuer_ura);
            if !user_keys.is_empty() {
                return Ok(user_keys
                    .iter()
                    .map(|entry| entry.public_key_b64.clone())
                    .collect());
            }
            trust_anchor.lookup(issuer_ura).map_or_else(
                || {
                    Err(Status::permission_denied(format!(
                        "{REASON_AUTHORITY_ISSUER_UNKNOWN}: authority issuer `{issuer_ura}` has no canonical invocation key"
                    )))
                },
                |issuer| Ok(vec![issuer.public_key_b64.clone()]),
            )
        },
    )
}

fn reject_unverified_runtime_authority_metadata(
    metadata: Option<&HashMap<String, String>>,
    ingress: &str,
) -> Result<(), Status> {
    let carries_runtime_authority = metadata.is_some_and(|metadata| {
        [DELEGATION_METADATA_KEY, SESSION_AUTHORITY_METADATA_KEY]
            .iter()
            .any(|key| {
                metadata
                    .get(*key)
                    .is_some_and(|value| !value.trim().is_empty())
            })
    });
    if carries_runtime_authority {
        return Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: {ingress} cannot carry authority metadata that this ingress does not verify"
        )));
    }
    Ok(())
}

fn reject_independent_authority_carriers(
    metadata: Option<&HashMap<String, String>>,
    ingress: &str,
) -> Result<(), Status> {
    reject_unverified_runtime_authority_metadata(metadata, ingress)?;
    let carries_authority_proof = metadata
        .and_then(|values| values.get(AUTHORITY_PROOF_METADATA_KEY))
        .is_some_and(|value| !value.trim().is_empty());
    if carries_authority_proof {
        return Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: {ingress} carries multiple independent authority proofs"
        )));
    }
    Ok(())
}

fn verify_local_system_authority_metadata(
    envelope: &Envelope,
    ability: &str,
    action: AccessAction,
    metadata: Option<&HashMap<String, String>>,
    now_ms: i64,
) -> Result<Option<VerifiedRuntimeAuthority>, Status> {
    // `TrustedLocalSystem` is a transport-proven ingress class, not a caller
    // URA inferred from public input. When that trusted ingress carries no
    // delegation/session metadata, the caller's daemon-custodied signature
    // and exact `_system.local` identity are the authority proof; the caller
    // projects `self_authority` after this function returns. External signed
    // requests never enter this wrapper and retain the generic fail-closed
    // `AUTHORITY_REQUIRED` rule below.
    let carries_authority_metadata = metadata.is_some_and(|metadata| {
        [DELEGATION_METADATA_KEY, SESSION_AUTHORITY_METADATA_KEY]
            .iter()
            .any(|key| {
                metadata
                    .get(*key)
                    .is_some_and(|value| !value.trim().is_empty())
            })
    });
    if !carries_authority_metadata {
        return Ok(None);
    }
    verify_authority_metadata_with_issuer_key(
        envelope,
        ability,
        action,
        metadata,
        now_ms,
        RuntimeAuthorityIssuerPolicy::LocalSystem,
        &|issuer_ura| {
            if issuer_ura != crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_UNKNOWN}: local-system authority issuer must be exact `{}`",
                    crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
                )));
            }
            let verifying_key =
                crate::daemon::identity::local_invocation::system_verifying_key().map_err(
                    |error| {
                        Status::failed_precondition(format!(
                            "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: local-system authority key unavailable: {error}"
                        ))
                    },
                )?;
            Ok(vec![BASE64_STANDARD.encode(verifying_key.to_bytes())])
        },
    )
}

fn verify_authority_metadata_with_issuer_key(
    envelope: &Envelope,
    ability: &str,
    action: AccessAction,
    metadata: Option<&HashMap<String, String>>,
    now_ms: i64,
    issuer_policy: RuntimeAuthorityIssuerPolicy,
    resolve_issuer_keys_b64: &dyn Fn(&str) -> Result<Vec<String>, Status>,
) -> Result<Option<VerifiedRuntimeAuthority>, Status> {
    let raw_delegation = metadata.and_then(|m| {
        m.get(DELEGATION_METADATA_KEY)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
    });
    let raw_session = metadata.and_then(|m| {
        m.get(SESSION_AUTHORITY_METADATA_KEY)
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
    });

    match (raw_delegation, raw_session) {
        (Some(_), Some(_)) => Err(Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: invocation carries both `{DELEGATION_METADATA_KEY}` \
                 and `{SESSION_AUTHORITY_METADATA_KEY}`"
        ))),
        (Some(raw_proof), None) => {
            let verified = parse_and_verify_delegation_proof_with_issuer_key(
                raw_proof,
                now_ms,
                resolve_issuer_keys_b64,
            )?;
            verify_delegation_bindings(&verified.payload, envelope, ability)?;
            verify_delegation_issuer_authorized(&verified.payload, envelope, issuer_policy)?;
            VerifiedRuntimeAuthority::delegated(verified).map(Some)
        }
        (None, Some(raw_session)) => {
            let verified = parse_and_verify_session_authority_with_issuer_key(
                raw_session,
                now_ms,
                resolve_issuer_keys_b64,
            )?;
            verify_session_authority_bindings(&verified.payload, envelope, ability, action)?;
            verify_session_issuer_authorized(
                &verified.payload,
                envelope,
                ability,
                action,
                issuer_policy,
            )?;
            VerifiedRuntimeAuthority::session(verified).map(Some)
        }
        (None, None) => {
            if envelope_requires_authority(envelope, ability) {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_REQUIRED}: envelope subject differs from caller and is a user/session/descriptor-bound authority subject; \
                     missing `{DELEGATION_METADATA_KEY}` or `{SESSION_AUTHORITY_METADATA_KEY}` metadata"
                )));
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
fn parse_and_verify_session_authority(
    raw_authority: &str,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
) -> Result<VerifiedSignedAuthority<SessionAuthorityPayload>, Status> {
    parse_and_verify_session_authority_with_issuer_key(raw_authority, now_ms, &|issuer_ura| {
        trust_anchor
                .lookup(issuer_ura)
                .map(|issuer| vec![issuer.public_key_b64.clone()])
                .ok_or_else(|| {
                    Status::permission_denied(format!(
                        "{REASON_AUTHORITY_ISSUER_UNKNOWN}: session authority issuer `{issuer_ura}` is not in the realm trust anchor"
                    ))
                })
    })
}

fn parse_and_verify_session_authority_with_issuer_key(
    raw_authority: &str,
    now_ms: i64,
    resolve_issuer_keys_b64: &dyn Fn(&str) -> Result<Vec<String>, Status>,
) -> Result<VerifiedSignedAuthority<SessionAuthorityPayload>, Status> {
    let wire = authority_metadata::decode_session_authority_wire(raw_authority)
        .map_err(authority_metadata_error_status)?;
    let payload = wire.payload;
    authority_metadata::validate_session_authority_payload_shape(&payload, Some(now_ms))
        .map_err(authority_metadata_error_status)?;

    let payload_bytes = authority_metadata::canonical_authority_payload_bytes(&payload)
        .map_err(authority_metadata_error_status)?;
    let signature = BASE64_STANDARD.decode(&wire.signature).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority signature base64 decode failed: {err}"
        ))
    })?;

    let issuer_public_keys_b64 = resolve_issuer_keys_b64(&payload.issuer_ura)?;
    verify_authority_signature_with_any_issuer_key(
        &issuer_public_keys_b64,
        &payload_bytes,
        &signature,
    )?;

    Ok(VerifiedSignedAuthority {
        payload,
        canonical_payload: payload_bytes,
        signature,
    })
}

fn verify_session_authority_bindings(
    payload: &SessionAuthorityPayload,
    envelope: &Envelope,
    ability: &str,
    action: AccessAction,
) -> Result<(), Status> {
    let ability_view = AuthorityAbilityView::from_envelope(envelope, ability)?;
    let caller = caller_ura_required(envelope)?;
    if payload.issuer_ura != caller {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_CALLER_MISMATCH}: session issuer `{}` does not match envelope \
             caller `{caller}`",
            payload.issuer_ura
        )));
    }

    let subject = subject_ura_required(envelope)?;
    if !authority_metadata::session_authority_admits_subject(payload, subject) {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SUBJECT_MISMATCH}: session subject `{}` does not exactly match \
             envelope subject `{subject}`",
            payload.subject_ura
        )));
    }

    let callee = callee_ura_required(envelope)?;
    if payload.callee_ura != callee {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_AUDIENCE_VIOLATION}: session callee `{}` does not match envelope \
             callee `{callee}`",
            payload.callee_ura
        )));
    }
    if !authority_metadata::authority_audience_admits(&payload.audience, callee) {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_AUDIENCE_VIOLATION}: session audience `{}` does not admit \
             envelope callee `{callee}`",
            payload.audience
        )));
    }

    if !payload
        .allowed_actions
        .iter()
        .any(|allowed| allowed.trim() == action.as_str())
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SCOPE_VIOLATION}: session allowed actions {:?} do not admit action \
             `{}`",
            payload.allowed_actions,
            action.as_str()
        )));
    }

    if !payload
        .allowed_followup_abilities
        .iter()
        .any(|candidate| ability_view.matches(candidate))
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SCOPE_VIOLATION}: session follow-up abilities {:?} do not admit \
             ability `{}`",
            payload.allowed_followup_abilities,
            ability_view.diagnostic_name()
        )));
    }

    if !payload
        .scopes
        .iter()
        .any(|pattern| ability_view.matches(pattern))
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SCOPE_VIOLATION}: session scopes {:?} do not admit ability \
             `{}`",
            payload.scopes,
            ability_view.diagnostic_name()
        )));
    }

    Ok(())
}

fn verify_delegation_issuer_authorized(
    payload: &DelegationPayload,
    envelope: &Envelope,
    issuer_policy: RuntimeAuthorityIssuerPolicy,
) -> Result<(), Status> {
    match issuer_policy {
        RuntimeAuthorityIssuerPolicy::LocalSystem => {
            verify_local_system_metadata_issuer(&payload.issuer_ura, envelope)
        }
        RuntimeAuthorityIssuerPolicy::RealmTrustAnchor => {
            verify_realm_scoped_authority_tuple(
                &payload.issuer_ura,
                &payload.caller_ura,
                callee_ura_required(envelope)?,
                &payload.subject_ura,
            )?;
            if let Some(sponsor_device_ura) =
                delegation_subject_device_sponsor_ura(&payload.subject_ura)?
            {
                if payload.issuer_ura == sponsor_device_ura
                    || payload.issuer_ura == payload.subject_ura
                {
                    return Ok(());
                }
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: delegation issuer `{}` is not the sponsoring Device `{sponsor_device_ura}` or exact SystemAgent `{}` for subject `{}`",
                    payload.issuer_ura, payload.subject_ura, payload.subject_ura
                )));
            }
            let owner_user_ura = delegation_subject_owner_user_ura(&payload.subject_ura)?;
            let issuer_user_ura = canonical_user_issuer_ura(&payload.issuer_ura)?;
            if issuer_user_ura != owner_user_ura {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: delegation issuer `{}` is not the owner `{owner_user_ura}` for subject `{}`",
                    payload.issuer_ura, payload.subject_ura
                )));
            }
            Ok(())
        }
    }
}

fn delegation_subject_device_sponsor_ura(subject_ura: &str) -> Result<Option<String>, Status> {
    let subject = parse_authority_runtime_ura("subject_ura", subject_ura)?;
    let Some((device_id, _system_agent_id)) = subject.device_agent_ids() else {
        return Ok(None);
    };
    Ok(Some(crate::core::ura::device_ura(
        &subject.realm,
        device_id,
    )))
}

fn verify_session_issuer_authorized(
    payload: &SessionAuthorityPayload,
    envelope: &Envelope,
    ability: &str,
    action: AccessAction,
    issuer_policy: RuntimeAuthorityIssuerPolicy,
) -> Result<(), Status> {
    match issuer_policy {
        RuntimeAuthorityIssuerPolicy::LocalSystem => {
            verify_local_system_metadata_issuer(&payload.issuer_ura, envelope)
        }
        RuntimeAuthorityIssuerPolicy::RealmTrustAnchor => {
            let issuer = parse_authority_runtime_ura("issuer_ura", &payload.issuer_ura)?;
            if issuer.kind == URAKind::Authority {
                if RealmAuthorityAdapterProfile::from_session_id(&payload.session_id)
                    == Some(RealmAuthorityAdapterProfile::PeerRuntimeInvocation)
                {
                    verify_peer_realm_authority_tuple(
                        &payload.issuer_ura,
                        caller_ura_required(envelope)?,
                        &payload.callee_ura,
                        &payload.subject_ura,
                    )?;
                } else {
                    verify_realm_scoped_authority_tuple(
                        &payload.issuer_ura,
                        caller_ura_required(envelope)?,
                        &payload.callee_ura,
                        &payload.subject_ura,
                    )?;
                }
                return verify_realm_authority_adapter(payload, envelope, ability, action);
            }
            let issuer_user_ura = canonical_user_issuer_ura(&payload.issuer_ura)?;
            if payload.creator_principal_id != issuer_user_ura {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: session creator `{}` must equal canonical issuer `{issuer_user_ura}`",
                    payload.creator_principal_id
                )));
            }
            let subject_kind = authority_metadata::authority_subject_kind(&payload.subject_ura);
            if subject_kind == AuthoritySubjectKind::Resource {
                return match authorize_user_session_device_resource(
                    UserSessionDeviceResourceTuple {
                        issuer_ura: &payload.issuer_ura,
                        caller_ura: caller_ura_required(envelope)?,
                        session_owner_user_id: &payload.session_owner_user_id,
                        callee_ura: callee_ura_required(envelope)?,
                        subject_ura: &payload.subject_ura,
                    },
                ) {
                    LocalDeviceResourceAuthorityDecision::Authorized => Ok(()),
                    LocalDeviceResourceAuthorityDecision::Denied(reason) => {
                        Err(Status::permission_denied(format!(
                            "{REASON_AUTHORITY_ISSUER_DENIED}: User-issued SessionAuthority cannot authorize Device Resource: {reason}"
                        )))
                    }
                };
            }
            if subject_kind == AuthoritySubjectKind::Agent {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: User-issued SessionAuthority cannot authorize Agent subjects"
                )));
            }
            verify_user_session_authority_tuple(
                &payload.issuer_ura,
                caller_ura_required(envelope)?,
                &payload.callee_ura,
                &payload.subject_ura,
            )?;
            let owner_user_ura = session_owner_user_ura(payload)?;
            if issuer_user_ura != owner_user_ura {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: session issuer `{}` is not the owner `{owner_user_ura}` for session `{}`",
                    payload.issuer_ura, payload.session_id
                )));
            }
            Ok(())
        }
    }
}

const REALM_AUTHORITY_ADAPTER_MAX_TTL_MS: i64 = 5 * 60 * 1_000;
const REALM_PRINCIPAL_ADAPTER_SESSION_ID_PREFIX: &str = "realm-account-adapter-";
const REALM_IDENTITY_ADAPTER_SESSION_ID_PREFIX: &str = "realm-identity-adapter-";
const REALM_DIRECTORY_READ_ADAPTER_SESSION_ID_PREFIX: &str = "realm-directory-read-adapter-";
const REALM_RUNTIME_INVOCATION_ADAPTER_SESSION_ID_PREFIX: &str =
    "realm-runtime-invocation-adapter-";
const REALM_PEER_RUNTIME_INVOCATION_ADAPTER_SESSION_ID_PREFIX: &str =
    "realm-peer-runtime-invocation-adapter-";
const AUTHORITY_GOVERNANCE_ADAPTER_SESSION_ID_PREFIX: &str = "authority-governance-adapter-";
const AGENT_GOVERNANCE_ADAPTER_SESSION_ID_PREFIX: &str = "agent-governance-adapter-";
const REALM_RECEIPT_HISTORY_ADAPTER_SESSION_ID_PREFIX: &str = "realm-receipt-history-adapter-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RealmAuthorityAdapterProfile {
    PrincipalLifecycle,
    IdentityRegistration,
    DirectoryRead,
    RuntimeInvocation,
    PeerRuntimeInvocation,
    AuthorityGovernance,
    AgentGovernance,
    ReceiptHistory,
    LifecycleSession,
}

impl RealmAuthorityAdapterProfile {
    fn from_session_id(session_id: &str) -> Option<Self> {
        if session_id.starts_with(REALM_PRINCIPAL_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::PrincipalLifecycle)
        } else if session_id.starts_with(REALM_IDENTITY_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::IdentityRegistration)
        } else if session_id.starts_with(REALM_DIRECTORY_READ_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::DirectoryRead)
        } else if session_id.starts_with(REALM_RUNTIME_INVOCATION_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::RuntimeInvocation)
        } else if session_id.starts_with(REALM_PEER_RUNTIME_INVOCATION_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::PeerRuntimeInvocation)
        } else if session_id.starts_with(AUTHORITY_GOVERNANCE_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::AuthorityGovernance)
        } else if session_id.starts_with(AGENT_GOVERNANCE_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::AgentGovernance)
        } else if session_id.starts_with(REALM_RECEIPT_HISTORY_ADAPTER_SESSION_ID_PREFIX) {
            Some(Self::ReceiptHistory)
        } else {
            None
        }
    }

    fn from_payload(payload: &SessionAuthorityPayload) -> Option<Self> {
        if let Some(profile) = Self::from_session_id(&payload.session_id) {
            return Some(profile);
        }
        let subject = crate::core::ura::parse_ura(&payload.subject_ura).ok()?;
        let session_path = subject.resource_path()?.strip_prefix("session/")?;
        (subject.kind == URAKind::Resource
            && !session_path.is_empty()
            && session_path == payload.session_id)
            .then_some(Self::LifecycleSession)
    }

    fn admits_ability(self, ability: &str) -> bool {
        match self {
            Self::PrincipalLifecycle => is_realm_account_adapter_principal_ability(ability),
            Self::IdentityRegistration => ability == ABILITY_IDENTITY_REGISTER_PUBKEY,
            Self::DirectoryRead => ability == federation::NAMESPACE_RESOLVE,
            Self::RuntimeInvocation => is_realm_runtime_invocation_adapter_ability(ability),
            Self::PeerRuntimeInvocation => is_realm_runtime_invocation_adapter_ability(ability),
            Self::AuthorityGovernance => is_realm_authority_governance_adapter_ability(ability),
            Self::AgentGovernance => is_agent_governance_adapter_ability(ability),
            Self::ReceiptHistory => is_receipt_history_adapter_ability(ability),
            Self::LifecycleSession => is_lifecycle_session_adapter_ability(ability),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PrincipalLifecycle => "PrincipalLifecycle",
            Self::IdentityRegistration => "IdentityRegistration",
            Self::DirectoryRead => "DirectoryRead",
            Self::RuntimeInvocation => "RuntimeInvocation",
            Self::PeerRuntimeInvocation => "PeerRuntimeInvocation",
            Self::AuthorityGovernance => "AuthorityGovernance",
            Self::AgentGovernance => "AgentGovernance",
            Self::ReceiptHistory => "ReceiptHistory",
            Self::LifecycleSession => "LifecycleSession",
        }
    }
}

fn verify_realm_authority_adapter(
    payload: &SessionAuthorityPayload,
    envelope: &Envelope,
    ability: &str,
    action: AccessAction,
) -> Result<(), Status> {
    let caller = caller_ura_required(envelope)?;
    let callee = callee_ura_required(envelope)?;
    if caller != payload.issuer_ura || payload.creator_principal_id != payload.issuer_ura {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter requires issuer, caller, and creator to equal the canonical realm Authority"
        )));
    }

    let ability_view = AuthorityAbilityView::from_envelope(envelope, ability)?;
    let public_name = ability_view.public_name.as_str();
    let profile = RealmAuthorityAdapterProfile::from_payload(payload).ok_or_else(|| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter session id has no recognized typed adapter discriminator"
        ))
    })?;
    match profile {
        RealmAuthorityAdapterProfile::RuntimeInvocation
        | RealmAuthorityAdapterProfile::PeerRuntimeInvocation
        | RealmAuthorityAdapterProfile::AgentGovernance
        | RealmAuthorityAdapterProfile::ReceiptHistory
        | RealmAuthorityAdapterProfile::LifecycleSession => {
            if payload.callee_ura != callee || payload.audience != callee {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter RuntimeInvocation profile requires exact Agent or Service callee and audience"
                )));
            }
            let parsed_callee = parse_authority_runtime_ura("callee_ura", callee)?;
            if !matches!(parsed_callee.kind, URAKind::Agent | URAKind::Service) {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter RuntimeInvocation callee must be an Agent, device-sponsored SystemAgent, or Service"
                )));
            }
        }
        _ => {
            if callee != payload.issuer_ura
                || payload.callee_ura != payload.issuer_ura
                || payload.audience != payload.issuer_ura
            {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter {} profile requires callee and audience to equal the canonical realm Authority",
                    profile.label()
                )));
            }
        }
    }
    if !profile.admits_ability(public_name) {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter {} profile cannot authorize ability `{public_name}`",
            profile.label()
        )));
    }
    if payload.scopes.len() != 1 || payload.scopes[0].trim() != public_name {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter scope must equal exact ability `{public_name}`"
        )));
    }
    if payload.allowed_followup_abilities.len() != 1
        || payload.allowed_followup_abilities[0].trim() != public_name
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter follow-up ability must equal exact ability `{public_name}`"
        )));
    }
    let expected_action = match profile {
        RealmAuthorityAdapterProfile::RuntimeInvocation
        | RealmAuthorityAdapterProfile::PeerRuntimeInvocation
        | RealmAuthorityAdapterProfile::AuthorityGovernance
        | RealmAuthorityAdapterProfile::AgentGovernance
        | RealmAuthorityAdapterProfile::ReceiptHistory
        | RealmAuthorityAdapterProfile::LifecycleSession => action,
        _ => admission_action_for(public_name).ok_or_else(|| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter ability `{public_name}` has no governed descriptor action"
            ))
        })?
        .into(),
    };
    if action.as_str() != expected_action.as_str()
        || payload.allowed_actions.len() != 1
        || payload.allowed_actions[0].trim() != expected_action.as_str()
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter action must equal exact descriptor action `{}`",
            expected_action.as_str()
        )));
    }
    let ttl_ms = payload.expires_at_ms.saturating_sub(payload.issued_at_ms);
    if ttl_ms <= 0 || ttl_ms > REALM_AUTHORITY_ADAPTER_MAX_TTL_MS {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter TTL exceeds {}ms",
            REALM_AUTHORITY_ADAPTER_MAX_TTL_MS
        )));
    }

    let subject = parse_authority_runtime_ura("subject_ura", &payload.subject_ura)?;
    let runtime_agent_subject = profile == RealmAuthorityAdapterProfile::RuntimeInvocation
        && subject.kind == URAKind::Agent
        && payload.subject_ura == callee;
    if subject.kind != URAKind::Resource && !runtime_agent_subject {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter subject must be a canonical Resource or the exact local RuntimeInvocation Agent"
        )));
    }
    if matches!(
        profile,
        RealmAuthorityAdapterProfile::RuntimeInvocation
            | RealmAuthorityAdapterProfile::PeerRuntimeInvocation
    ) {
        if payload.session_owner_user_id.trim().is_empty() {
            return Err(Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter RuntimeInvocation requires an accountable session owner User"
            )));
        }
    } else if profile == RealmAuthorityAdapterProfile::AuthorityGovernance
        && subject.resource_owner_id() == Some("authority")
    {
        let subject_path = subject.resource_path().unwrap_or_default();
        let exact_ability_subject = subject_path
            .strip_prefix("invoke/")
            .is_some_and(|value| value == public_name && !value.contains('/'));
        if !exact_ability_subject {
            return Err(Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter AuthorityGovernance subject must exactly bind Authority ability `{public_name}`"
            )));
        }
    } else {
        let owner_user_id = subject
            .resource_owner_id()
            .and_then(|owner| owner.strip_prefix("user."))
            .filter(|owner| !owner.is_empty() && !owner.contains('.'))
            .ok_or_else(|| {
                Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter subject must be a canonical User-owned resource"
                ))
            })?;
        let subject_path = subject.resource_path().unwrap_or_default();
        let subject_matches = match profile {
            RealmAuthorityAdapterProfile::LifecycleSession => subject_path
                .strip_prefix("session/")
                .is_some_and(|session_id| {
                    !session_id.is_empty() && session_id == payload.session_id
                }),
            RealmAuthorityAdapterProfile::AuthorityGovernance
            | RealmAuthorityAdapterProfile::AgentGovernance
            | RealmAuthorityAdapterProfile::ReceiptHistory => {
                subject_path == "runtime-state/read"
                    || subject_path
                        .strip_prefix("invoke/")
                        .is_some_and(|value| value == public_name && !value.contains('/'))
            }
            _ => subject_path
                .strip_prefix("invoke/")
                .is_some_and(|value| value == public_name && !value.contains('/')),
        };
        if owner_user_id != payload.session_owner_user_id || !subject_matches {
            return Err(Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_DENIED}: RealmAuthorityAdapter subject owner and ability must match session owner `{}` and ability `{public_name}`",
                payload.session_owner_user_id
            )));
        }
    }
    Ok(())
}

fn is_realm_account_adapter_principal_ability(ability: &str) -> bool {
    matches!(
        ability,
        governance::PRINCIPAL_CREATE
            | governance::PRINCIPAL_BIND_FIRST_KEY
            | governance::PRINCIPAL_ADD_KEY
            | governance::PRINCIPAL_ROTATE_KEY
            | governance::PRINCIPAL_REVOKE_KEY
            | governance::PRINCIPAL_CONFIGURE_RECOVERY
            | governance::PRINCIPAL_RECOVER
            | governance::PRINCIPAL_SUSPEND
            | governance::PRINCIPAL_REACTIVATE
            | governance::PRINCIPAL_DELETE
            | governance::PRINCIPAL_ISSUE_ENROLLMENT
            | governance::PRINCIPAL_REVOKE_ENROLLMENT
            | governance::PRINCIPAL_ISSUE_GRANT
            | governance::PRINCIPAL_REVOKE_GRANT
            | governance::PRINCIPAL_GET
    )
}

fn is_realm_runtime_invocation_adapter_ability(ability: &str) -> bool {
    let ability = ability.trim();
    !ability.is_empty()
        && !ability.starts_with("federation.")
        && !ability.starts_with("namespace.")
        && !ability.starts_with("identity.")
        && !ability.starts_with("principal.")
        && !ability.starts_with("runtime.")
}

fn is_realm_authority_governance_adapter_ability(ability: &str) -> bool {
    admission_action_for(ability).is_some() || ability == "admin.status"
}

fn is_agent_governance_adapter_ability(ability: &str) -> bool {
    matches!(
        ability.trim(),
        "meta.list_abilities" | "meta.list_resources"
    )
}

fn is_receipt_history_adapter_ability(ability: &str) -> bool {
    let ability = ability.trim();
    ability.starts_with("invocation.history.") || ability.starts_with("invocation.trace.")
}

fn is_lifecycle_session_adapter_ability(ability: &str) -> bool {
    matches!(ability.trim(), "terminal.attach" | "terminal.close")
}

fn verify_local_system_metadata_issuer(
    issuer_ura: &str,
    envelope: &Envelope,
) -> Result<(), Status> {
    let caller = caller_ura_required(envelope)?;
    if issuer_ura != crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
        || caller != crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: local-system runtime authority requires exact `_system.local` issuer and caller"
        )));
    }
    Ok(())
}

fn verify_realm_scoped_authority_tuple(
    issuer_ura: &str,
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
) -> Result<(), Status> {
    let issuer = parse_authority_runtime_ura("issuer_ura", issuer_ura)?;
    let caller = parse_authority_runtime_ura("caller_ura", caller_ura)?;
    let callee = parse_authority_runtime_ura("callee_ura", callee_ura)?;
    let subject = parse_authority_runtime_ura("subject_ura", subject_ura)?;
    for (field, realm) in [
        ("caller_ura", caller.realm.as_str()),
        ("callee_ura", callee.realm.as_str()),
        ("subject_ura", subject.realm.as_str()),
    ] {
        if realm != issuer.realm {
            return Err(Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_DENIED}: authority {field} realm `{realm}` does not match issuer realm `{}`",
                issuer.realm
            )));
        }
    }
    Ok(())
}

/// Verify the realm geometry of a User-issued SessionAuthority.
///
/// A User-owned Resource remains in the User's origin realm even when the
/// exact descriptor-owning Agent is hosted in a peer realm. Signature trust
/// for that external User is established before this function through the
/// configured federated resolver or the destination Device's bounded
/// Hub-attested key projection. This function only validates the signed tuple;
/// it does not manufacture peer trust or let either Hub replace the User.
fn verify_user_session_authority_tuple(
    issuer_ura: &str,
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
) -> Result<(), Status> {
    let issuer = parse_authority_runtime_ura("issuer_ura", issuer_ura)?;
    let caller = parse_authority_runtime_ura("caller_ura", caller_ura)?;
    let callee = parse_authority_runtime_ura("callee_ura", callee_ura)?;
    let subject = parse_authority_runtime_ura("subject_ura", subject_ura)?;
    if issuer.kind != URAKind::User || caller.kind != URAKind::User || caller_ura != issuer_ura {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: User SessionAuthority issuer and caller must be the exact canonical User"
        )));
    }
    if subject.realm != issuer.realm {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: User SessionAuthority subject realm `{}` does not match issuer realm `{}`",
            subject.realm, issuer.realm
        )));
    }
    if callee.realm == issuer.realm {
        return Ok(());
    }
    if callee.kind != URAKind::Agent {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: cross-realm User SessionAuthority callee must be an exact Agent"
        )));
    }
    Ok(())
}

fn verify_peer_realm_authority_tuple(
    issuer_ura: &str,
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
) -> Result<(), Status> {
    let issuer = parse_authority_runtime_ura("issuer_ura", issuer_ura)?;
    let caller = parse_authority_runtime_ura("caller_ura", caller_ura)?;
    let callee = parse_authority_runtime_ura("callee_ura", callee_ura)?;
    let subject = parse_authority_runtime_ura("subject_ura", subject_ura)?;
    if issuer.kind != URAKind::Authority || caller.kind != URAKind::Authority {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: peer runtime adapter issuer and caller must be the local realm Authority"
        )));
    }
    if caller_ura != issuer_ura || caller.realm != issuer.realm {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: peer runtime adapter caller must exactly equal its issuer"
        )));
    }
    if subject.realm != issuer.realm {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: peer runtime adapter subject must remain in issuer realm `{}`",
            issuer.realm
        )));
    }
    if callee.kind != URAKind::Agent || callee.realm == issuer.realm {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: peer runtime adapter callee must be an Agent in a distinct peer realm"
        )));
    }
    Ok(())
}

fn canonical_user_issuer_ura(issuer_ura: &str) -> Result<String, Status> {
    let issuer = parse_authority_runtime_ura("issuer_ura", issuer_ura)?;
    if issuer.kind != URAKind::User {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: runtime authority issuer `{issuer_ura}` must be a canonical User URA for user-owned subjects"
        )));
    }
    let user_id = issuer.user_id().ok_or_else(|| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_DENIED}: runtime authority issuer `{issuer_ura}` has no User id"
        ))
    })?;
    Ok(crate::core::ura::user_ura(&issuer.realm, user_id))
}

fn session_owner_user_ura(payload: &SessionAuthorityPayload) -> Result<String, Status> {
    let subject = parse_authority_runtime_ura("subject_ura", &payload.subject_ura)?;
    Ok(crate::core::ura::user_ura(
        &subject.realm,
        payload.session_owner_user_id.as_str(),
    ))
}

fn delegation_subject_owner_user_ura(subject_ura: &str) -> Result<String, Status> {
    let subject = parse_authority_runtime_ura("subject_ura", subject_ura)?;
    let owner_user_id = match subject.kind {
        URAKind::User => subject.user_id().map(ToOwned::to_owned).ok_or_else(|| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_DENIED}: delegation subject `{subject_ura}` has no User owner id"
            ))
        })?,
        URAKind::Agent => {
            if let Some((user_id, _)) = subject.agent_ids() {
                user_id.to_string()
            } else if subject.device_agent_ids().is_some() {
                return Err(delegation_subject_requires_non_user_authority(
                    subject_ura,
                    "device-sponsored SystemAgent subjects require Device/SystemAgent authority",
                ));
            } else {
                return Err(delegation_subject_requires_non_user_authority(
                    subject_ura,
                    "Agent subject owner could not be resolved to a canonical User",
                ));
            }
        }
        URAKind::Service => subject
            .service_ids()
            .map(|(principal_id, _)| principal_id.to_string())
            .ok_or_else(|| {
                Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_DENIED}: delegation subject `{subject_ura}` has no Service principal id"
                ))
            })?,
        URAKind::Ability => {
            let selector = AbilitySelector::parse(subject_ura).map_err(|error| {
                Status::invalid_argument(format!(
                    "{REASON_AUTHORITY_FORMAT_INVALID}: authority subject Ability URA `{subject_ura}` is not canonical: {error}"
                ))
            })?;
            return delegation_subject_owner_user_ura(selector.owner_ura());
        }
        URAKind::Resource => {
            let owner_id = subject.resource_owner_id().ok_or_else(|| {
                delegation_subject_requires_non_user_authority(
                    subject_ura,
                    "Resource subject omitted its owner id",
                )
            })?;
            resource_owner_user_id(owner_id)
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    delegation_subject_requires_non_user_authority(
                        subject_ura,
                        "Resource subject owner is not a canonical User-owned resource",
                    )
                })?
        }
        URAKind::Device => {
            return Err(delegation_subject_requires_non_user_authority(
                subject_ura,
                "Device subjects require Device/SystemAgent authority",
            ));
        }
        URAKind::Authority => {
            return Err(delegation_subject_requires_non_user_authority(
                subject_ura,
                "Authority subjects require RealmAuthority authority",
            ));
        }
        URAKind::Unknown => {
            return Err(delegation_subject_requires_non_user_authority(
                subject_ura,
                "subject kind is unknown",
            ));
        }
    };
    Ok(crate::core::ura::user_ura(&subject.realm, &owner_user_id))
}

fn resource_owner_user_id(owner_id: &str) -> Option<&str> {
    if let Some(user_id) = owner_id.strip_prefix("user.") {
        return (!user_id.is_empty() && !user_id.contains('.')).then_some(user_id);
    }
    owner_id
        .strip_prefix("agent.")
        .and_then(|rest| rest.split_once('.').map(|(user_id, _)| user_id))
        .filter(|user_id| !user_id.is_empty())
}

fn delegation_subject_requires_non_user_authority(subject_ura: &str, detail: &str) -> Status {
    Status::permission_denied(format!(
        "{REASON_AUTHORITY_ISSUER_DENIED}: delegation subject `{subject_ura}` cannot be authorized by a User issuer: {detail}"
    ))
}

fn parse_authority_runtime_ura(
    field: &'static str,
    value: &str,
) -> Result<crate::core::ura::ParsedURA, Status> {
    let value = value.trim();
    parse_ura(value).map_err(|error| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority {field} `{value}` is not canonical: {error}"
        ))
    })
}

#[derive(Debug, Clone)]
struct AuthorityAbilityView {
    wire: String,
    public_name: String,
    ability_ura: String,
}

impl AuthorityAbilityView {
    fn from_envelope(envelope: &Envelope, ability: &str) -> Result<Self, Status> {
        let callee_ura = envelope
            .callee
            .as_ref()
            .map(|callee| callee.ura.as_str())
            .map(str::trim)
            .filter(|callee| !callee.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "authority ability projection requires envelope callee URA",
                )
            })?;
        let wire = ability.trim();
        if wire.is_empty() {
            return Err(Status::invalid_argument(
                "authority ability projection requires ability",
            ));
        }
        let ability_ura =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, wire)
                .map_err(axon_error_to_status)?;
        let public_name = public_name_from_authority_ability_ura(&ability_ura)?;
        Ok(Self {
            wire: wire.to_string(),
            public_name,
            ability_ura,
        })
    }

    fn diagnostic_name(&self) -> &str {
        if self.public_name.is_empty() {
            &self.wire
        } else {
            &self.public_name
        }
    }

    fn matches(&self, pattern: &str) -> bool {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return false;
        }
        scope_matches(pattern, &self.public_name)
            || scope_matches(pattern, &self.ability_ura)
            || scope_matches(pattern, &self.wire)
    }
}

fn public_name_from_authority_ability_ura(ability_ura: &str) -> Result<String, Status> {
    AbilitySelector::parse(ability_ura)
        .map(|selector| selector.public_name().to_string())
        .map_err(|error| {
            Status::invalid_argument(format!(
                "authority ability projection derived non-canonical ability URA `{ability_ura}`: {error}"
            ))
        })
}

fn verified_session_authority_id(payload: &SessionAuthorityPayload) -> String {
    format!("session_authority:{}", payload.session_id)
}

fn verified_delegation_authority_id(payload: &DelegationPayload) -> Result<String, Status> {
    let payload_bytes = authority_metadata::canonical_authority_payload_bytes(payload)
        .map_err(authority_metadata_error_status)?;
    Ok(format!(
        "delegation:sha256:{}",
        hex::encode(sha2::Sha256::digest(payload_bytes))
    ))
}

#[cfg(test)]
fn parse_and_verify_delegation_proof(
    raw_proof: &str,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
) -> Result<VerifiedSignedAuthority<DelegationPayload>, Status> {
    parse_and_verify_delegation_proof_with_issuer_key(raw_proof, now_ms, &|issuer_ura| {
        trust_anchor
                .lookup(issuer_ura)
                .map(|issuer| vec![issuer.public_key_b64.clone()])
                .ok_or_else(|| {
                    Status::permission_denied(format!(
                        "{REASON_AUTHORITY_ISSUER_UNKNOWN}: authority issuer `{issuer_ura}` is not in the realm trust anchor"
                    ))
                })
    })
}

fn parse_and_verify_delegation_proof_with_issuer_key(
    raw_proof: &str,
    now_ms: i64,
    resolve_issuer_keys_b64: &dyn Fn(&str) -> Result<Vec<String>, Status>,
) -> Result<VerifiedSignedAuthority<DelegationPayload>, Status> {
    let wire = authority_metadata::decode_delegation_authority_wire(raw_proof)
        .map_err(authority_metadata_error_status)?;
    let payload = wire.payload;
    authority_metadata::validate_delegation_payload_shape(&payload, Some(now_ms))
        .map_err(authority_metadata_error_status)?;

    let payload_bytes = authority_metadata::canonical_authority_payload_bytes(&payload)
        .map_err(authority_metadata_error_status)?;
    let signature = BASE64_STANDARD.decode(&wire.signature).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority signature base64 decode failed: {err}"
        ))
    })?;

    let issuer_public_keys_b64 = resolve_issuer_keys_b64(&payload.issuer_ura)?;
    verify_authority_signature_with_any_issuer_key(
        &issuer_public_keys_b64,
        &payload_bytes,
        &signature,
    )?;

    Ok(VerifiedSignedAuthority {
        payload,
        canonical_payload: payload_bytes,
        signature,
    })
}

fn envelope_requires_authority(envelope: &Envelope, ability: &str) -> bool {
    let Some(caller) = envelope.caller.as_ref().map(|c| c.ura.as_str()) else {
        return false;
    };
    let Some(subject) = envelope.subject.as_ref().map(|s| s.ura.as_str()) else {
        return false;
    };
    if caller == subject {
        return false;
    }
    if device_sponsored_system_agent_publication(envelope, ability) {
        return false;
    }
    matches!(
        authority_metadata::authority_subject_kind(subject),
        AuthoritySubjectKind::User
            | AuthoritySubjectKind::Service
            | AuthoritySubjectKind::Agent
            | AuthoritySubjectKind::Session
            | AuthoritySubjectKind::DescriptorBound
            | AuthoritySubjectKind::RuntimeStateRead
            | AuthoritySubjectKind::Resource
    )
}

fn reject_host_local_permission_probe_target_resource_subject(
    descriptor: &BoundAdmissionDescriptor,
    envelope: &Envelope,
    ability: &str,
) -> Result<(), Status> {
    if descriptor.subject_contract_ura.as_deref()
        != Some(
            crate::daemon::plugins::package::REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA,
        )
    {
        return Ok(());
    }
    let Some(subject_ura) = envelope.subject.as_ref().map(|subject| subject.ura.trim()) else {
        return Ok(());
    };
    let subject = parse_ura(subject_ura).map_err(|error| {
        Status::invalid_argument(format!(
            "host-local permission probe subject_ura is not canonical: {error}"
        ))
    })?;
    if !host_local_permission_probe_target_resource_subject(&subject) {
        return Ok(());
    }
    Err(Status::invalid_argument(format!(
        "{ability}: screen-capture permission probes are host-local and MUST NOT be scoped to a remote desktop resource subject; reason=invalid_argument"
    )))
}

fn host_local_permission_probe_target_resource_subject(
    subject: &crate::core::ura::ParsedURA,
) -> bool {
    subject.kind == URAKind::Resource
        && (subject
            .resource_owner_id()
            .is_some_and(|owner| owner.starts_with("device."))
            || subject
                .resource_path()
                .is_some_and(|path| path.starts_with("streams/")))
}

/// A Device is the cryptographic sponsor and host of its declared native
/// SystemAgents. That exact relationship authorizes only the pre-session
/// owner-projection publication needed to establish the live catalog; it is
/// not a general Device-as-Agent invocation exemption.
fn device_sponsored_system_agent_publication(envelope: &Envelope, ability: &str) -> bool {
    let Ok(ability_view) = AuthorityAbilityView::from_envelope(envelope, ability) else {
        return false;
    };
    if !ability_view
        .matches(crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES)
    {
        return false;
    }
    let Some(caller) = envelope
        .caller
        .as_ref()
        .and_then(|identity| parse_ura(identity.ura.trim()).ok())
    else {
        return false;
    };
    let Some(subject) = envelope
        .subject
        .as_ref()
        .and_then(|identity| parse_ura(identity.ura.trim()).ok())
    else {
        return false;
    };
    let Some(caller_device_id) = caller.device_id() else {
        return false;
    };
    subject
        .device_agent_ids()
        .is_some_and(|(sponsor_device_id, system_agent_id)| {
            caller.realm == subject.realm
                && sponsor_device_id == caller_device_id
                && crate::daemon::ability::catalog::profiles::is_declared_daemon_native_system_agent_id(
                    system_agent_id,
                )
        })
}

fn verify_authority_signature_with_any_issuer_key(
    issuer_public_keys_b64: &[String],
    payload_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), Status> {
    if issuer_public_keys_b64.is_empty() {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: authority issuer has no active verification key"
        )));
    }
    if issuer_public_keys_b64
        .iter()
        .any(|key| verify_delegation_signature(key, payload_bytes, signature_bytes).is_ok())
    {
        return Ok(());
    }
    Err(Status::permission_denied(format!(
        "{REASON_AUTHORITY_SIGNATURE_INVALID}: authority signature does not verify against any active issuer key"
    )))
}

fn verify_delegation_signature(
    issuer_public_key_b64: &str,
    payload_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), Status> {
    let public_key = BASE64_STANDARD
        .decode(issuer_public_key_b64)
        .map_err(|err| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: issuer public key is not valid base64: {err}"
            ))
        })?;
    let key_bytes: [u8; ed25519_dalek::PUBLIC_KEY_LENGTH] =
        public_key.as_slice().try_into().map_err(|_| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: issuer public key wrong size, want {} got {}",
                ed25519_dalek::PUBLIC_KEY_LENGTH,
                public_key.len()
            ))
        })?;
    let signature_bytes: [u8; ed25519_dalek::SIGNATURE_LENGTH] =
        signature_bytes.try_into().map_err(|_| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_SIGNATURE_INVALID}: signature wrong size, want {} got {}",
                ed25519_dalek::SIGNATURE_LENGTH,
                signature_bytes.len()
            ))
        })?;
    let verifying_key = VerifyingKey::from_bytes(&key_bytes).map_err(|err| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_KEY_NOT_FOUND}: issuer public key rejected: {err}"
        ))
    })?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(payload_bytes, &signature)
        .map_err(|err| {
            Status::permission_denied(format!(
                "{REASON_AUTHORITY_SIGNATURE_INVALID}: authority signature does not verify: {err}"
            ))
        })
}

fn authority_metadata_error_status(err: AuthorityMetadataError) -> Status {
    match err.reason() {
        REASON_AUTHORITY_EXPIRED => Status::permission_denied(err.status_message()),
        _ => Status::invalid_argument(err.status_message()),
    }
}

/// Shallow envelope cross-checks for a presented DelegatedBy claim.
///
/// Per the frozen compatibility matrix
/// (document/rfcs/001-authority-binding-relation-evidence.md in
/// EasyNet-Axon), DelegatedBy no longer has a `caller_ura`/`subject_ura`
/// equality requirement at this layer:
///   - the old `caller_ura` field no longer exists — the SDK's signature
///     verification over the claim bytes binds `envelope.caller` as
///     delegatee directly (a stronger check than a plain string equality
///     here, since it's cryptographically bound, not just asserted);
///   - the old `subject_ura == envelope.subject` check contradicted the
///     RFC's own archaeology finding: `Delegated.subject_ura` (now
///     `binding.authority`) is an independent dimension from
///     `envelope.subject`, never cross-checked historically or by the
///     SDK's own shallow/deep gates. Removed here so this daemon-side
///     gate is consistent with what the SDK now enforces, rather than a
///     second, contradictory gate.
///
/// Audience and scope checks remain — they are still real, meaningful
/// requirements independent of this correction.
fn verify_delegation_bindings(
    payload: &DelegationPayload,
    envelope: &Envelope,
    ability: &str,
) -> Result<(), Status> {
    let callee = callee_ura_required(envelope)?;
    if !authority_metadata::authority_audience_admits(&payload.audience, callee) {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_AUDIENCE_VIOLATION}: authority audience `{}` does not admit \
             envelope callee `{callee}`",
            payload.audience
        )));
    }

    if !payload
        .scopes
        .iter()
        .any(|pattern| scope_matches(pattern, ability))
    {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SCOPE_VIOLATION}: authority scopes {:?} do not admit ability \
             `{ability}`",
            payload.scopes
        )));
    }

    Ok(())
}

fn scope_matches(pattern: &str, ability: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return !prefix.is_empty() && ability.starts_with(prefix);
    }
    pattern == ability
}

// Phase 5a deleted `hex_lower` — its sole caller was the
// `record_admission_receipt` helper, which built the
// `invocation_id` string for SharedReceiptStore entries. The
// store + helper are gone; nothing else needed lowercase-hex
// of the 16-byte nonce.

/// Extract `caller.ura` and reject as `invalid_argument` if absent
/// or empty. Shared by every entrypoint so the wire-level
/// "caller URA required" message is identical across surfaces.
fn caller_ura_required(envelope: &Envelope) -> Result<&str, Status> {
    required_envelope_identity_ura(
        envelope.caller.as_ref().map(|caller| caller.ura.as_str()),
        "caller",
        "caller URA required",
    )
}

/// Extract `callee.ura` and reject as `invalid_argument` if absent
/// or empty. Authority verification must not synthesize an audience
/// from an incomplete canonical tuple.
fn callee_ura_required(envelope: &Envelope) -> Result<&str, Status> {
    required_envelope_identity_ura(
        envelope.callee.as_ref().map(|callee| callee.ura.as_str()),
        "callee",
        "callee URA required",
    )
}

/// Extract `subject.ura` and reject as `invalid_argument` if absent
/// or empty. Authority verification must compare explicit subject
/// facts rather than treating a missing subject as an empty owner.
fn subject_ura_required(envelope: &Envelope) -> Result<&str, Status> {
    required_envelope_identity_ura(
        envelope
            .subject
            .as_ref()
            .map(|subject| subject.ura.as_str()),
        "subject",
        "subject URA required",
    )
}

fn required_envelope_identity_ura<'a>(
    value: Option<&'a str>,
    field: &str,
    invariant: &str,
) -> Result<&'a str, Status> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{REASON_ENVELOPE_INCOMPLETE}: envelope.{field}.ura is required \
             (Invariant 1: {invariant})"
            ))
        })?;
    crate::core::identity::RuntimeIdentityUra::parse(value).map_err(|error| {
        Status::invalid_argument(format!(
            "{REASON_ENVELOPE_INCOMPLETE}: envelope.{field}.ura {error}"
        ))
    })?;
    Ok(value)
}

fn current_unix_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn permission_denied_unknown_caller(caller_ura: &str) -> Status {
    Status::permission_denied(format!(
        "{}: caller URA `{caller_ura}` is not in the canonical local \
         PrincipalLifecycle aggregate or realm trust-anchor projection",
        SignatureDecisionReason::CallerKeyNotFound.as_str(),
    ))
}

fn principal_admission_state_label(state: PrincipalAdmissionState) -> &'static str {
    match state {
        PrincipalAdmissionState::Missing => "missing",
        PrincipalAdmissionState::Pending => "pending",
        PrincipalAdmissionState::Active => "active",
        PrincipalAdmissionState::Suspended => "suspended",
        PrincipalAdmissionState::Deleted => "deleted",
    }
}

fn reject_public_hosted_agent_delegation_metadata(
    metadata: Option<&HashMap<String, String>>,
) -> Result<(), Status> {
    let Some(rejected_key) = [
        HOSTED_AGENT_DELEGATION_METADATA_KEY,
        HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY,
    ]
    .into_iter()
    .find(|key| {
        metadata
            .and_then(|m| m.get(*key))
            .map(String::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) else {
        return Ok(());
    };

    Err(Status::permission_denied(format!(
        "{REASON_HOSTED_AGENT_DELEGATION_LOCAL_ONLY}: `{rejected_key}` \
         is local daemon control metadata and is only accepted on local self admission ingress"
    )))
}

/// Map an axon-SDK invocation `AxonError` (the kind admission
/// emits) to a `tonic::Status`. The mapping preserves the canonical
/// reason (e.g. `CALLER_SIGNATURE_INVALID`) inside the status
/// message so audit pipelines and client-side metrics that grep on
/// those strings continue to work.
fn axon_error_to_status(err: InvocationError) -> Status {
    let detail = if err.message.is_empty() {
        err.reason.clone()
    } else {
        format!("{}:{}", err.reason, err.message)
    };
    match err.reason.as_str() {
        REASON_ENVELOPE_INCOMPLETE => Status::invalid_argument(detail),
        REASON_CALLER_SIGNATURE_INVALID => Status::invalid_argument(detail),
        REASON_NONCE_REPLAY => Status::invalid_argument(detail),
        _ => match err.kind {
            InvocationErrorKind::InvalidArgument => Status::invalid_argument(detail),
            InvocationErrorKind::PermissionDenied => Status::permission_denied(detail),
            InvocationErrorKind::ResourceExhausted => Status::resource_exhausted(detail),
            InvocationErrorKind::Unavailable => Status::unavailable(detail),
            InvocationErrorKind::DeadlineExceeded => Status::deadline_exceeded(detail),
            InvocationErrorKind::Cancelled => Status::cancelled(detail),
            InvocationErrorKind::Internal => Status::internal(detail),
        },
    }
}

fn ensure_signed_descriptor_ref_matches_route(
    envelope: &Envelope,
    route_ability: &str,
    descriptor_ref: &str,
) -> Result<(), Status> {
    let callee_ura = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.as_str())
        .map(str::trim)
        .filter(|callee| !callee.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument("signed descriptor ref route check requires envelope callee")
        })?;
    let signed_ability_ura =
        crate::daemon::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(
            callee_ura,
            descriptor_ref,
        )
        .map_err(axon_error_to_status)
        .and_then(|canonical_ref| {
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                &canonical_ref,
            )
            .map_err(axon_error_to_status)
        })?;
    let routed_ability_ura =
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, route_ability)
            .map_err(axon_error_to_status)?;
    if signed_ability_ura != routed_ability_ura {
        return Err(Status::invalid_argument(format!(
            "{}: signed descriptor ref `{descriptor_ref}` does not match route `{route_ability}` \
             for callee `{callee_ura}`",
            SignatureDecisionReason::SignedDescriptorRefMismatch.as_str()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::federation::client::{FederationClient, FederationClientError, HubEndpoint};
    use crate::daemon::federation::peers::SharedFederatedPeers;
    use crate::daemon::trust::cell::SharedTrustAnchor;
    use axon_sdk::invocation::{
        sha256, AgentIdentity, CausalContext, InvocationEnvelope, SubjectIdentity, UraProfile,
    };
    use axon_sdk::pb::axon::v1::{
        AgentIdentity as PbAgentIdentity, InvokeRequest, InvokeResponse,
        SubjectIdentity as PbSubjectIdentity,
    };
    use ed25519_dalek::{Signer as _, SigningKey};
    use serde_json::json;
    use std::collections::{BTreeMap, HashMap};
    use tempfile::tempdir;

    const TEST_DESCRIPTOR_REF: &str = "easynet:///r/policy/ability/policy.worker.run@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke";

    #[test]
    fn system_agent_route_preserves_public_namespace_matching_owner_id() {
        let owner = crate::core::ura::device_agent_ura("hub", "node-a", "terminal");

        assert_eq!(
            public_ability_name_from_route_for_owner(&owner, "terminal.create"),
            "terminal.create"
        );
    }

    struct StaticInvocationVerificationKeyProvider {
        caller_ura: String,
        verifying_key: ed25519_dalek::VerifyingKey,
    }

    impl crate::daemon::identity::receipt_signing::InvocationVerificationKeyProvider
        for StaticInvocationVerificationKeyProvider
    {
        fn resolve_invocation_verifying_key(
            &self,
            caller_ura: &str,
        ) -> Result<Option<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
            Ok((caller_ura == self.caller_ura).then_some(self.verifying_key))
        }
    }

    #[test]
    fn runtime_policy_denial_projects_canonical_pre_admission_facts() {
        let error = runtime_admission_status_to_axon(Status::permission_denied(
            r#"POLICY_DENIED: {"decision":"deny","reason":"OWNER_UNRESOLVED"}"#,
        ));

        assert_eq!(error.code, ErrorCode::AbilityForbidden);
        assert_eq!(error.stage, Some(ErrorStage::AbilityPolicy));
        assert_eq!(error.security_class, Some(SecurityClass::Authorization));
        assert_eq!(error.reason, ErrorCode::AbilityForbidden.as_str());
        assert!(
            error.message.contains("OWNER_UNRESOLVED"),
            "policy detail must remain visible for operator diagnosis: {error:?}"
        );
    }

    #[test]
    fn runtime_authority_denial_preserves_subject_mismatch_fact() {
        let error = runtime_admission_status_to_axon(Status::permission_denied(
            "AUTHORITY_DENIED: AUTHORITY_SUBJECT_MISMATCH: session subject does not admit envelope subject",
        ));

        assert_eq!(error.code, ErrorCode::AuthoritySubjectMismatch);
        assert_eq!(error.stage, Some(ErrorStage::AuthorityValidation));
        assert_eq!(error.security_class, Some(SecurityClass::Authority));
        assert_eq!(error.reason, ErrorCode::AuthoritySubjectMismatch.as_str());
    }

    #[test]
    fn runtime_quota_denial_projects_resource_stage() {
        let error = runtime_admission_status_to_axon(Status::resource_exhausted(
            "QUOTA_EXCEEDED: caller=easynet:///r/policy/user/alice ability=demo retry_after_ms=1000",
        ));

        assert_eq!(error.code, ErrorCode::QuotaExceeded);
        assert_eq!(error.stage, Some(ErrorStage::Quota));
        assert_eq!(error.security_class, Some(SecurityClass::Resource));
    }

    fn receipt_policy_envelope(
        caller_ura: &str,
        callee_ura: &str,
        subject_ura: &str,
    ) -> DescriptorBoundEnvelope {
        DescriptorBoundEnvelope::new(InvocationEnvelope {
            caller: AgentIdentity::new(caller_ura, UraProfile::StrictV2),
            callee: AgentIdentity::new(callee_ura, UraProfile::StrictV2),
            subject: SubjectIdentity::new(subject_ura, UraProfile::StrictV2),
            ability: TEST_DESCRIPTOR_REF.to_string(),
            args_digest: sha256(b"{}"),
            invocation_nonce: [0x42; 16],
            causal_context: CausalContext::None,
        })
        .expect("test receipt policy envelope must be descriptor-bound")
    }

    fn authority_wire_envelope(
        caller_ura: Option<&str>,
        callee_ura: Option<&str>,
        subject_ura: Option<&str>,
    ) -> Envelope {
        Envelope {
            caller: caller_ura.map(|ura| PbAgentIdentity {
                ura: ura.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            callee: callee_ura.map(|ura| PbAgentIdentity {
                ura: ura.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            subject: subject_ura.map(|ura| PbSubjectIdentity {
                ura: ura.to_string(),
                profile: "axon-strict-v2".to_string(),
            }),
            ..Envelope::default()
        }
    }

    fn bound_read_descriptor_with_subject_contract(
        subject_contract_ura: Option<&str>,
    ) -> BoundAdmissionDescriptor {
        BoundAdmissionDescriptor {
            owner_ura: "easynet:///r/example/agent/device.dev-a.remote-desktop".to_string(),
            hosted_agent_device_ura: None,
            action: AccessAction::Read,
            safe_read: true,
            subject_contract_ura: subject_contract_ura.map(str::to_string),
        }
    }

    fn local_system_terminal_authority_request(
        now_ms: i64,
    ) -> authority_metadata::SessionAuthorityRequest {
        let issuer = crate::core::ura::LOCAL_SYSTEM_AGENT_URA;
        authority_metadata::SessionAuthorityRequest {
            issuer_ura: issuer.to_string(),
            session_id: "session-1".to_string(),
            session_owner_user_id: "alice".to_string(),
            creator_principal_id: issuer.to_string(),
            callee_ura: "easynet:///r/example/agent/device.dev-a.terminal".to_string(),
            subject_ura: "easynet:///r/example/resource/user.alice/session/session-1".to_string(),
            audience: "easynet:///r/example/agent/device.dev-a.terminal".to_string(),
            scopes: vec!["terminal.*".to_string()],
            allowed_actions: vec!["invoke".to_string()],
            allowed_followup_abilities: vec!["terminal.read".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        }
    }

    fn signed_delegation_metadata(
        payload: DelegationPayload,
        signing_key: &SigningKey,
    ) -> HashMap<String, String> {
        let canonical_payload =
            authority_metadata::canonical_authority_payload_bytes(&payload).expect("payload");
        let signature = signing_key.sign(&canonical_payload).to_bytes();
        let wire = json!({
            "payload": payload,
            "signature": BASE64_STANDARD.encode(signature),
        });
        HashMap::from([(
            DELEGATION_METADATA_KEY.to_string(),
            BASE64_STANDARD.encode(crate::daemon::ability::canonical_json_bytes(&wire)),
        )])
    }

    fn signed_session_metadata(
        request: authority_metadata::SessionAuthorityRequest,
        signing_key: &SigningKey,
    ) -> HashMap<String, String> {
        authority_metadata::CanonicalSessionAuthorityIssuer::issue(
            request.clone(),
            &request.issuer_ura,
            |canonical| {
                Ok::<_, std::convert::Infallible>(signing_key.sign(canonical).to_bytes().to_vec())
            },
        )
        .expect("issue session authority")
        .into_map()
    }

    fn signed_session_metadata_unchecked(
        request: authority_metadata::SessionAuthorityRequest,
        signing_key: &SigningKey,
    ) -> HashMap<String, String> {
        let payload = SessionAuthorityPayload::from(request);
        let canonical_payload =
            authority_metadata::canonical_authority_payload_bytes(&payload).expect("payload");
        let signature = signing_key.sign(&canonical_payload).to_bytes();
        let wire = json!({
            "payload": payload,
            "signature": BASE64_STANDARD.encode(signature),
        });
        HashMap::from([(
            SESSION_AUTHORITY_METADATA_KEY.to_string(),
            BASE64_STANDARD.encode(crate::daemon::ability::canonical_json_bytes(&wire)),
        )])
    }

    fn issuer_key_resolver<'a>(
        expected_issuer: &'a str,
        signing_key: &'a SigningKey,
    ) -> impl Fn(&str) -> Result<Vec<String>, Status> + 'a {
        move |issuer_ura| {
            if issuer_ura == expected_issuer {
                Ok(vec![
                    BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes())
                ])
            } else {
                Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_ISSUER_UNKNOWN}: unexpected test issuer `{issuer_ura}`"
                )))
            }
        }
    }

    fn require_authority_metadata_error(
        result: Result<Option<VerifiedRuntimeAuthority>, Status>,
        message: &str,
    ) -> Status {
        match result {
            Ok(_) => panic!("{message}"),
            Err(error) => error,
        }
    }

    #[test]
    fn trusted_local_system_verifies_session_authority_before_runtime_projection() {
        let now_ms = current_unix_ms();
        let request = local_system_terminal_authority_request(now_ms);
        let issued = authority_metadata::CanonicalSessionAuthorityIssuer::issue(
            request.clone(),
            crate::core::ura::LOCAL_SYSTEM_AGENT_URA,
            |canonical| {
                crate::daemon::identity::local_invocation::sign_system_canonical(canonical)
                    .map(|signature| signature.to_bytes().to_vec())
            },
        )
        .expect("issue local-system session authority");
        let metadata = issued.into_map();
        let envelope = authority_wire_envelope(
            Some(crate::core::ura::LOCAL_SYSTEM_AGENT_URA),
            Some(&request.callee_ura),
            Some(&request.subject_ura),
        );

        let authority = verify_local_system_authority_metadata(
            &envelope,
            "terminal.read",
            AccessAction::Invoke,
            Some(&metadata),
            now_ms,
        )
        .expect("local-system authority must verify")
        .expect("session authority must be projected");

        assert_eq!(
            authority.authority_id(),
            Some("session_authority:session-1")
        );
        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn trusted_local_system_without_metadata_allows_typed_capability_projection() {
        let envelope = authority_wire_envelope(
            Some(crate::core::ura::LOCAL_SYSTEM_AGENT_URA),
            Some("easynet:///r/example/agent/device.node-a.locomotion"),
            Some("easynet:///r/example/resource/camera-1"),
        );

        let authority = verify_local_system_authority_metadata(
            &envelope,
            "camera.snapshot",
            AccessAction::Invoke,
            None,
            current_unix_ms(),
        )
        .expect("trusted local-system ingress must preserve its typed authority class");

        assert!(authority.is_none());
    }

    #[test]
    fn external_user_without_metadata_still_requires_authority() {
        let caller = "easynet:///r/example/user/alice";
        let subject = crate::core::ura::resource_dot_ura("example", "user.alice", "invoke/fs.read");
        let envelope = authority_wire_envelope(
            Some(caller),
            Some("easynet:///r/example/agent/device.node-a.locomotion"),
            Some(&subject),
        );
        let resolver = |_issuer_ura: &str| -> Result<Vec<String>, Status> {
            panic!("missing metadata must reject before resolving an issuer key")
        };

        let error = match verify_authority_metadata_with_issuer_key(
            &envelope,
            "fs.read",
            AccessAction::Read,
            None,
            current_unix_ms(),
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        ) {
            Ok(_) => panic!("external User ingress without authority must remain fail-closed"),
            Err(error) => error,
        };

        assert_eq!(error.code(), Code::PermissionDenied);
        assert!(
            error.message().contains(REASON_AUTHORITY_REQUIRED),
            "{error}"
        );
    }

    #[test]
    fn cross_realm_user_route_target_remains_rejected_after_key_resolution() {
        let caller = "easynet:///r/example/user/alice";
        let callee = "easynet:///r/peer.example/agent/device.node-a.locomotion";
        let envelope = authority_wire_envelope(Some(caller), Some(callee), Some(callee));
        let resolver = |_issuer_ura: &str| -> Result<Vec<String>, Status> {
            panic!("missing authority metadata must reject independently of caller-key readiness")
        };

        let error = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                "shell.run",
                AccessAction::Invoke,
                None,
                current_unix_ms(),
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "direct User route-target invocation must not gain authority from key synchronization",
        );

        assert_eq!(error.code(), Code::PermissionDenied);
        assert!(
            error.message().contains(REASON_AUTHORITY_REQUIRED),
            "{error}"
        );
    }

    #[test]
    fn external_authority_descriptor_subject_without_metadata_requires_authority() {
        let authority = "easynet:///r/example/authority";
        let subject = crate::core::ura::resource_dot_ura(
            "example",
            "authority",
            "invoke/federation.subscribe_directory_v2",
        );
        let envelope = authority_wire_envelope(Some(authority), Some(authority), Some(&subject));
        let resolver = |_issuer_ura: &str| -> Result<Vec<String>, Status> {
            panic!("missing metadata must reject before resolving an issuer key")
        };

        let error = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                "federation.subscribe_directory_v2",
                AccessAction::Stream,
                None,
                current_unix_ms(),
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "Authority-owned descriptor subject without proof must fail closed",
        );

        assert_eq!(error.code(), Code::PermissionDenied);
        assert!(
            error.message().contains(REASON_AUTHORITY_REQUIRED),
            "{error}"
        );
    }

    #[test]
    fn typed_authority_ingresses_reject_independent_authority_carriers() {
        for ingress in [
            "peer-directory-stream admission",
            "runtime-derived-child admission",
        ] {
            for key in [
                DELEGATION_METADATA_KEY,
                SESSION_AUTHORITY_METADATA_KEY,
                AUTHORITY_PROOF_METADATA_KEY,
            ] {
                let metadata = HashMap::from([(key.to_string(), "non-empty-proof".to_string())]);
                let error = reject_independent_authority_carriers(Some(&metadata), ingress)
                    .expect_err("typed authority ingress must be the only authority carrier");

                assert_eq!(
                    error.code(),
                    Code::InvalidArgument,
                    "{ingress} {key}: {error}"
                );
                assert!(
                    error.message().contains(REASON_AUTHORITY_FORMAT_INVALID),
                    "{ingress} {key}: {error}"
                );
            }
        }
    }

    #[test]
    fn trusted_local_system_rejects_unverified_session_authority() {
        let now_ms = current_unix_ms();
        let request = local_system_terminal_authority_request(now_ms);
        let issued = authority_metadata::CanonicalSessionAuthorityIssuer::prepare(
            request.clone(),
            crate::core::ura::LOCAL_SYSTEM_AGENT_URA,
        )
        .expect("prepare authority")
        .seal(vec![0x00; ed25519_dalek::SIGNATURE_LENGTH])
        .expect("encode invalid authority fixture");
        let metadata = issued.into_map();
        let envelope = authority_wire_envelope(
            Some(crate::core::ura::LOCAL_SYSTEM_AGENT_URA),
            Some(&request.callee_ura),
            Some(&request.subject_ura),
        );

        let error = match verify_local_system_authority_metadata(
            &envelope,
            "terminal.read",
            AccessAction::Invoke,
            Some(&metadata),
            now_ms,
        ) {
            Ok(_) => panic!("unverified local-system authority must fail closed"),
            Err(error) => error,
        };

        assert_eq!(error.code(), Code::PermissionDenied);
        assert!(
            error.message().contains(REASON_AUTHORITY_SIGNATURE_INVALID),
            "{error}"
        );
    }

    #[test]
    fn sponsor_device_authorizes_only_its_declared_system_agent_projection() {
        let caller = "easynet:///r/example/device/dev-a";
        let hub = "easynet:///r/example/authority";
        let own_system_agent = "easynet:///r/example/agent/device.dev-a.a2a-integration";
        let foreign_system_agent = "easynet:///r/example/agent/device.dev-b.a2a-integration";
        let undeclared_system_agent = "easynet:///r/example/agent/device.dev-a.not-declared";

        let own = authority_wire_envelope(Some(caller), Some(hub), Some(own_system_agent));
        assert!(!envelope_requires_authority(
            &own,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        ));
        assert!(envelope_requires_authority(&own, "observe.health"));

        for subject in [foreign_system_agent, undeclared_system_agent] {
            let envelope = authority_wire_envelope(Some(caller), Some(hub), Some(subject));
            assert!(envelope_requires_authority(
                &envelope,
                crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            ));
        }
    }

    #[test]
    fn user_owned_service_projection_requires_delegation_authority() {
        let device = "easynet:///r/example/device/dev-a";
        let hub = "easynet:///r/example/authority";
        let service = "easynet:///r/example/service/alice.pages";
        let envelope = authority_wire_envelope(Some(device), Some(hub), Some(service));

        assert!(envelope_requires_authority(
            &envelope,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
        ));
    }

    #[test]
    fn host_local_permission_contract_rejects_target_resource_subject_as_invalid_argument() {
        let descriptor = bound_read_descriptor_with_subject_contract(Some(
            crate::daemon::plugins::package::REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA,
        ));
        let envelope = authority_wire_envelope(
            Some("easynet:///r/example/user/alice"),
            Some("easynet:///r/example/agent/device.dev-a.remote-desktop"),
            Some("easynet:///r/example/resource/device.dev-a/streams/window.7"),
        );

        let error = reject_host_local_permission_probe_target_resource_subject(
            &descriptor,
            &envelope,
            "remote_desktop.permission_status",
        )
        .expect_err("target resource subject must be malformed for host-local permission probes");

        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(error.message().contains("MUST NOT be scoped"), "{error}");
        assert!(
            error.message().contains("reason=invalid_argument"),
            "{error}"
        );
    }

    #[test]
    fn ordinary_resource_ability_keeps_authority_required_for_target_resource_subject() {
        let descriptor = bound_read_descriptor_with_subject_contract(None);
        let envelope = authority_wire_envelope(
            Some("easynet:///r/example/user/alice"),
            Some("easynet:///r/example/agent/device.dev-a.media"),
            Some("easynet:///r/example/resource/device.dev-a/streams/window.7"),
        );

        reject_host_local_permission_probe_target_resource_subject(
            &descriptor,
            &envelope,
            "screen.snapshot",
        )
        .expect("non permission-probe descriptors must not be reclassified");
        assert!(envelope_requires_authority(&envelope, "screen.snapshot"));
    }

    #[test]
    fn realm_trust_delegation_allows_user_issuer_for_own_subject() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/user/alice";
        let caller = "easynet:///r/example/user/alice";
        let callee = "easynet:///r/example/agent/service.worker";
        let subject = "easynet:///r/example/resource/user.alice/document/report";
        let signing_key = SigningKey::from_bytes(&[0x41; 32]);
        let payload = DelegationPayload {
            issuer_ura: issuer.to_string(),
            subject_ura: subject.to_string(),
            caller_ura: caller.to_string(),
            audience: callee.to_string(),
            scopes: vec!["run".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_delegation_metadata(payload, &signing_key);
        let envelope = authority_wire_envelope(Some(caller), Some(callee), Some(subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            "run",
            AccessAction::Invoke,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("same-owner delegation must verify")
        .expect("delegation authority must project");

        assert_eq!(authority.binding.form(), "delegated_by+delegation");
    }

    #[test]
    fn realm_trust_delegation_rejects_user_issuer_for_other_user_subject() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/user/mallory";
        let caller = "easynet:///r/example/user/mallory";
        let callee = "easynet:///r/example/agent/service.worker";
        let subject = "easynet:///r/example/resource/user.alice/document/report";
        let signing_key = SigningKey::from_bytes(&[0x42; 32]);
        let payload = DelegationPayload {
            issuer_ura: issuer.to_string(),
            subject_ura: subject.to_string(),
            caller_ura: caller.to_string(),
            audience: callee.to_string(),
            scopes: vec!["run".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_delegation_metadata(payload, &signing_key);
        let envelope = authority_wire_envelope(Some(caller), Some(callee), Some(subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let err = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                "run",
                AccessAction::Invoke,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "Mallory must not self-issue delegation over Alice subject",
        );

        assert_eq!(err.code(), Code::PermissionDenied);
        assert!(
            err.message().contains(REASON_AUTHORITY_ISSUER_DENIED),
            "{err}"
        );
    }

    #[test]
    fn realm_trust_delegation_allows_user_issuer_for_user_owned_ability_subject() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/user/alice";
        let caller = "easynet:///r/example/user/alice";
        let callee = "easynet:///r/example/agent/service.worker";
        let subject = "easynet:///r/example/ability/alice.worker.run";
        let signing_key = SigningKey::from_bytes(&[0x46; 32]);
        let payload = DelegationPayload {
            issuer_ura: issuer.to_string(),
            subject_ura: subject.to_string(),
            caller_ura: caller.to_string(),
            audience: callee.to_string(),
            scopes: vec!["run".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_delegation_metadata(payload, &signing_key);
        let envelope = authority_wire_envelope(Some(caller), Some(callee), Some(subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            "run",
            AccessAction::Invoke,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("user-owned ability subject must verify")
        .expect("delegation authority must project");

        assert_eq!(authority.binding.form(), "delegated_by+delegation");
    }

    fn assert_realm_trust_delegation_rejects_alice_subject(subject: &str, message: &str) {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/user/alice";
        let caller = "easynet:///r/example/user/alice";
        let callee = "easynet:///r/example/agent/service.worker";
        let signing_key = SigningKey::from_bytes(&[0x47; 32]);
        let payload = DelegationPayload {
            issuer_ura: issuer.to_string(),
            subject_ura: subject.to_string(),
            caller_ura: caller.to_string(),
            audience: callee.to_string(),
            scopes: vec!["run".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_delegation_metadata(payload, &signing_key);
        let envelope = authority_wire_envelope(Some(caller), Some(callee), Some(subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let err = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                "run",
                AccessAction::Invoke,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            message,
        );

        assert_eq!(err.code(), Code::PermissionDenied);
        assert!(
            err.message().contains(REASON_AUTHORITY_ISSUER_DENIED),
            "{err}"
        );
    }

    #[test]
    fn realm_trust_delegation_rejects_user_issuer_for_device_subject() {
        assert_realm_trust_delegation_rejects_alice_subject(
            "easynet:///r/example/device/dev-a",
            "User issuer must not self-issue delegation over a Device subject",
        );
    }

    #[test]
    fn realm_trust_delegation_rejects_user_issuer_for_authority_subject() {
        assert_realm_trust_delegation_rejects_alice_subject(
            "easynet:///r/example/authority",
            "User issuer must not self-issue delegation over a RealmAuthority subject",
        );
    }

    #[test]
    fn realm_trust_delegation_rejects_user_issuer_for_device_sponsored_system_agent_subject() {
        let subject = crate::core::ura::device_agent_ura("example", "dev-a", "terminal");
        assert_realm_trust_delegation_rejects_alice_subject(
            &subject,
            "User issuer must not self-issue delegation over a device-sponsored SystemAgent subject",
        );
    }

    #[test]
    fn realm_trust_delegation_allows_sponsor_device_for_device_sponsored_system_agent_subject() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/device/dev-a";
        let caller = issuer;
        let callee = "easynet:///r/example/authority";
        let subject = crate::core::ura::device_agent_ura("example", "dev-a", "remote-desktop");
        let signing_key = SigningKey::from_bytes(&[0x48; 32]);
        let payload = DelegationPayload {
            issuer_ura: issuer.to_string(),
            subject_ura: subject.clone(),
            caller_ura: caller.to_string(),
            audience: callee.to_string(),
            scopes: vec![
                crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES
                    .to_string(),
            ],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_delegation_metadata(payload, &signing_key);
        let envelope = authority_wire_envelope(Some(caller), Some(callee), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            AccessAction::Manage,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("sponsor Device delegation over its SystemAgent subject must verify")
        .expect("delegation authority must project");

        assert_eq!(authority.binding.form(), "delegated_by+delegation");
    }

    #[test]
    fn realm_trust_delegation_rejects_user_issuer_for_device_owned_ability_subject() {
        assert_realm_trust_delegation_rejects_alice_subject(
            "easynet:///r/example/ability/device.dev-a.observe.health",
            "User issuer must not self-issue delegation over a Device-owned Ability subject",
        );
    }

    #[test]
    fn realm_trust_delegation_rejects_user_issuer_for_unsupported_resource_owner_subject() {
        let subject =
            crate::core::ura::resource_dot_ura("example", "device.dev-a.files", "tmp/report.txt");
        assert_realm_trust_delegation_rejects_alice_subject(
            &subject,
            "User issuer must not self-issue delegation over an unsupported Resource owner",
        );
    }

    #[test]
    fn realm_trust_session_rejects_user_issuer_for_other_owner() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/user/mallory";
        let callee = &crate::core::ura::device_agent_ura("example", "dev-a", "worker");
        let subject = "easynet:///r/example/resource/user.alice/session/session-1";
        let signing_key = SigningKey::from_bytes(&[0x43; 32]);
        let request = authority_metadata::SessionAuthorityRequest {
            issuer_ura: issuer.to_string(),
            session_id: "session-1".to_string(),
            session_owner_user_id: "alice".to_string(),
            creator_principal_id: issuer.to_string(),
            callee_ura: callee.to_string(),
            subject_ura: subject.to_string(),
            audience: callee.to_string(),
            scopes: vec!["run".to_string()],
            allowed_actions: vec!["invoke".to_string()],
            allowed_followup_abilities: vec!["run".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let err = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                "run",
                AccessAction::Invoke,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "Mallory must not self-issue Alice session authority",
        );

        assert_eq!(err.code(), Code::PermissionDenied, "{err}");
        assert!(
            err.message().contains(REASON_AUTHORITY_ISSUER_DENIED),
            "{err}"
        );
    }

    #[test]
    fn realm_trust_session_rejects_same_user_id_from_different_realm() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/realm-a/user/alice";
        let callee = &crate::core::ura::device_agent_ura("realm-a", "dev-a", "worker");
        let subject = "easynet:///r/realm-b/resource/user.alice/session/session-1";
        let signing_key = SigningKey::from_bytes(&[0x44; 32]);
        let request = authority_metadata::SessionAuthorityRequest {
            issuer_ura: issuer.to_string(),
            session_id: "session-1".to_string(),
            session_owner_user_id: "alice".to_string(),
            creator_principal_id: issuer.to_string(),
            callee_ura: callee.to_string(),
            subject_ura: subject.to_string(),
            audience: callee.to_string(),
            scopes: vec!["run".to_string()],
            allowed_actions: vec!["invoke".to_string()],
            allowed_followup_abilities: vec!["run".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let err = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                "run",
                AccessAction::Invoke,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "same user id from another Realm must not authorize session",
        );

        assert_eq!(err.code(), Code::PermissionDenied, "{err}");
        assert!(
            err.message().contains(REASON_AUTHORITY_ISSUER_DENIED),
            "{err}"
        );
    }

    #[test]
    fn realm_trust_session_allows_paired_user_for_exact_local_device_resource() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev-a".into(),
                credential_token: "token".into(),
                hub_endpoint: "https://hub.example".into(),
                realm: "example".into(),
                deploy_signature: String::new(),
                hub_api_base: None,
                username: Some("alice".into()),
                user_id: Some("alice".into()),
                hub_pubkey_b64: None,
                hub_tls_ca_pem_b64: None,
                join_receipt_hash: None,
            },
        )
        .expect("save paired credentials");

        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/user/alice";
        let subject =
            &crate::core::ura::resource_dot_ura("example", "device.dev-a", "streams/display.01");
        let signing_key = SigningKey::from_bytes(&[0x4d; 32]);
        let resolver = issuer_key_resolver(issuer, &signing_key);
        for (callee, ability, action, session_id) in [
            (
                crate::core::ura::device_agent_ura("example", "dev-a", "media"),
                "screen.snapshot",
                AccessAction::Read,
                "invoke-media-resource",
            ),
            (
                crate::core::ura::device_agent_ura(
                    "example",
                    "dev-a",
                    crate::daemon::ability::names::integrations::PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID,
                ),
                "remote_desktop.grant_consent",
                AccessAction::Manage,
                "invoke-remote-desktop-consent",
            ),
        ] {
            let request = authority_metadata::SessionAuthorityRequest {
                issuer_ura: issuer.to_string(),
                session_id: session_id.to_string(),
                session_owner_user_id: "alice".to_string(),
                creator_principal_id: issuer.to_string(),
                callee_ura: callee.clone(),
                subject_ura: subject.to_string(),
                audience: callee.clone(),
                scopes: vec![ability.to_string()],
                allowed_actions: vec![action.as_str().to_string()],
                allowed_followup_abilities: vec![ability.to_string()],
                issued_at_ms: now_ms - 1_000,
                expires_at_ms: now_ms + 60_000,
            };
            let metadata = signed_session_metadata(request, &signing_key);
            let envelope = authority_wire_envelope(Some(issuer), Some(&callee), Some(subject));

            verify_authority_metadata_with_issuer_key(
                &envelope,
                ability,
                action,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            )
            .unwrap_or_else(|error| {
                panic!("paired user must authorize {ability} on this Device Resource: {error}")
            });
        }
    }

    #[test]
    fn realm_trust_session_rejects_inexact_local_device_resource_ownership_tuples() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev-a".into(),
                credential_token: "token".into(),
                hub_endpoint: "https://hub.example".into(),
                realm: "example".into(),
                deploy_signature: String::new(),
                hub_api_base: None,
                username: Some("alice".into()),
                user_id: Some("alice".into()),
                hub_pubkey_b64: None,
                hub_tls_ca_pem_b64: None,
                join_receipt_hash: None,
            },
        )
        .expect("save paired credentials");

        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/user/alice";
        let signing_key = SigningKey::from_bytes(&[0x4e; 32]);
        let resolver = issuer_key_resolver(issuer, &signing_key);
        for (callee, subject, session_owner_user_id, creator_principal_id, label, session_suffix) in [
            (
                crate::core::ura::device_agent_ura("example", "dev-b", "plugin-management"),
                crate::core::ura::resource_dot_ura("example", "device.dev-a", "streams/display.01"),
                "alice",
                issuer,
                "callee sponsored by another Device",
                "foreign-callee-device",
            ),
            (
                "easynet:///r/example/agent/service.remote-desktop".to_string(),
                crate::core::ura::resource_dot_ura("example", "device.dev-a", "streams/display.01"),
                "alice",
                issuer,
                "hosted Agent callee",
                "hosted-agent",
            ),
            (
                crate::core::ura::device_agent_ura("example", "dev-a", "plugin-management"),
                crate::core::ura::resource_dot_ura("example", "device.dev-b", "streams/display.01"),
                "alice",
                issuer,
                "Resource owned by another Device",
                "foreign-resource-device",
            ),
            (
                crate::core::ura::device_agent_ura("example", "dev-a", "plugin-management"),
                crate::core::ura::resource_dot_ura("example", "device.dev-a", "streams/display.01"),
                "mallory",
                issuer,
                "session owned by another User",
                "foreign-session-owner",
            ),
            (
                crate::core::ura::device_agent_ura("example", "dev-a", "plugin-management"),
                crate::core::ura::resource_dot_ura("example", "device.dev-a", "streams/display.01"),
                "alice",
                "easynet:///r/example/user/mallory",
                "session created by another principal",
                "foreign-creator",
            ),
        ] {
            let ability = "remote_desktop.grant_consent";
            let action = AccessAction::Manage;
            let request = authority_metadata::SessionAuthorityRequest {
                issuer_ura: issuer.to_string(),
                session_id: format!("invoke-{session_suffix}"),
                session_owner_user_id: session_owner_user_id.to_string(),
                creator_principal_id: creator_principal_id.to_string(),
                callee_ura: callee.clone(),
                subject_ura: subject.clone(),
                audience: callee.clone(),
                scopes: vec![ability.to_string()],
                allowed_actions: vec![action.as_str().to_string()],
                allowed_followup_abilities: vec![ability.to_string()],
                issued_at_ms: now_ms - 1_000,
                expires_at_ms: now_ms + 60_000,
            };
            let metadata = signed_session_metadata(request, &signing_key);
            let envelope = authority_wire_envelope(Some(issuer), Some(&callee), Some(&subject));
            let err = require_authority_metadata_error(
                verify_authority_metadata_with_issuer_key(
                    &envelope,
                    ability,
                    action,
                    Some(&metadata),
                    now_ms,
                    RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                    &resolver,
                ),
                label,
            );

            assert_eq!(err.code(), Code::PermissionDenied);
            assert!(
                err.message().contains(REASON_AUTHORITY_ISSUER_DENIED),
                "{err}"
            );
        }
    }

    #[test]
    fn user_session_authority_admits_exact_foreign_agent_for_rpc_and_stream() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/realm-a/user/alice";
        let callee = "easynet:///r/realm-b/agent/device.node-b.locomotion";
        let subject = "easynet:///r/realm-a/resource/user.alice/invoke/shell.run";
        let signing_key = SigningKey::from_bytes(&[0x4a; 32]);
        let resolver = issuer_key_resolver(issuer, &signing_key);

        for action in [AccessAction::Invoke, AccessAction::Stream] {
            let request = authority_metadata::SessionAuthorityRequest {
                issuer_ura: issuer.to_string(),
                session_id: format!("session-cross-realm-{}", action.as_str()),
                session_owner_user_id: "alice".to_string(),
                creator_principal_id: issuer.to_string(),
                callee_ura: callee.to_string(),
                subject_ura: subject.to_string(),
                audience: callee.to_string(),
                scopes: vec!["shell.run".to_string()],
                allowed_actions: vec![action.as_str().to_string()],
                allowed_followup_abilities: vec!["shell.run".to_string()],
                issued_at_ms: now_ms - 1_000,
                expires_at_ms: now_ms + 60_000,
            };
            let metadata = signed_session_metadata(request, &signing_key);
            let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(subject));

            let authority = verify_authority_metadata_with_issuer_key(
                &envelope,
                "shell.run",
                action,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            )
            .expect("exact cross-realm User authority must verify")
            .expect("session authority must project");

            assert_eq!(authority.binding.form(), "session_of+session");
        }
    }

    #[test]
    fn user_session_authority_rejects_foreign_non_agent_callee() {
        let issuer = "easynet:///r/realm-a/user/alice";
        let foreign_authority = "easynet:///r/realm-b/authority";
        let subject = "easynet:///r/realm-a/resource/user.alice/invoke/shell.run";

        let error = verify_user_session_authority_tuple(issuer, issuer, foreign_authority, subject)
            .expect_err("User authority must not act as cross-realm RealmAuthority authority");

        assert_eq!(error.code(), Code::PermissionDenied);
        assert!(error.message().contains(REASON_AUTHORITY_ISSUER_DENIED));
    }

    #[test]
    fn user_session_authority_rejects_forged_cross_realm_issuer_signature() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/realm-a/user/alice";
        let callee = "easynet:///r/realm-b/agent/device.node-b.locomotion";
        let subject = "easynet:///r/realm-a/resource/user.alice/invoke/shell.run";
        let trusted_key = SigningKey::from_bytes(&[0x4b; 32]);
        let forged_key = SigningKey::from_bytes(&[0x4c; 32]);
        let request = authority_metadata::SessionAuthorityRequest {
            issuer_ura: issuer.to_string(),
            session_id: "session-cross-realm-forged".to_string(),
            session_owner_user_id: "alice".to_string(),
            creator_principal_id: issuer.to_string(),
            callee_ura: callee.to_string(),
            subject_ura: subject.to_string(),
            audience: callee.to_string(),
            scopes: vec!["shell.run".to_string()],
            allowed_actions: vec!["invoke".to_string()],
            allowed_followup_abilities: vec!["shell.run".to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        };
        let metadata = signed_session_metadata(request, &forged_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(subject));
        let resolver = issuer_key_resolver(issuer, &trusted_key);

        let error = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                "shell.run",
                AccessAction::Invoke,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "forged cross-realm issuer signature must fail closed",
        );

        assert_eq!(error.code(), Code::PermissionDenied);
        assert!(error.message().contains(REASON_AUTHORITY_SIGNATURE_INVALID));
    }

    fn realm_account_adapter_session_request(
        now_ms: i64,
        ability: &str,
        action: AccessAction,
    ) -> authority_metadata::SessionAuthorityRequest {
        let issuer = "easynet:///r/example/authority";
        authority_metadata::SessionAuthorityRequest {
            issuer_ura: issuer.to_string(),
            session_id: "realm-account-adapter-1".to_string(),
            session_owner_user_id: "alice".to_string(),
            creator_principal_id: issuer.to_string(),
            callee_ura: issuer.to_string(),
            subject_ura: crate::core::ura::resource_dot_ura(
                "example",
                "user.alice",
                &format!("invoke/{ability}"),
            ),
            audience: issuer.to_string(),
            scopes: vec![ability.to_string()],
            allowed_actions: vec![action.as_str().to_string()],
            allowed_followup_abilities: vec![ability.to_string()],
            issued_at_ms: now_ms - 1_000,
            expires_at_ms: now_ms + 60_000,
        }
    }

    #[test]
    fn realm_account_adapter_session_admits_exact_principal_lifecycle_operation() {
        let now_ms = current_unix_ms();
        let ability = governance::PRINCIPAL_GET;
        let issuer = "easynet:///r/example/authority";
        let signing_key = SigningKey::from_bytes(&[0x51; 32]);
        let request = realm_account_adapter_session_request(now_ms, ability, AccessAction::Read);
        let subject = request.subject_ura.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(issuer), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Read,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("exact RealmAccountAdapter PrincipalLifecycle authority must verify")
        .expect("session authority must project");

        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn realm_identity_adapter_session_admits_exact_device_key_registration() {
        let now_ms = current_unix_ms();
        let ability = ABILITY_IDENTITY_REGISTER_PUBKEY;
        let issuer = "easynet:///r/example/authority";
        let signing_key = SigningKey::from_bytes(&[0x53; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Manage);
        request.session_id = "realm-identity-adapter-1".to_string();
        let subject = request.subject_ura.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(issuer), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Manage,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("exact RealmIdentityAdapter registration authority must verify")
        .expect("session authority must project");

        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn realm_directory_read_adapter_admits_exact_namespace_resolve() {
        let now_ms = current_unix_ms();
        let ability = federation::NAMESPACE_RESOLVE;
        let issuer = "easynet:///r/example/authority";
        let signing_key = SigningKey::from_bytes(&[0x55; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Read);
        request.session_id = "realm-directory-read-adapter-1".to_string();
        let subject = request.subject_ura.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(issuer), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Read,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("exact RealmDirectoryReadAdapter authority must verify")
        .expect("session authority must project");

        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn realm_runtime_invocation_adapter_admits_exact_system_agent_tuple() {
        let now_ms = current_unix_ms();
        let ability = "fs.read";
        let issuer = "easynet:///r/example/authority";
        let callee = "easynet:///r/example/agent/device.node-a.runtime";
        let signing_key = SigningKey::from_bytes(&[0x56; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Read);
        request.session_id = "realm-runtime-invocation-adapter-1".to_string();
        request.callee_ura = callee.to_string();
        request.audience = callee.to_string();
        let subject = request.subject_ura.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Read,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("exact RealmRuntimeInvocationAdapter authority must verify")
        .expect("session authority must project");

        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn realm_runtime_invocation_adapter_admits_service_callee_tuple() {
        let now_ms = current_unix_ms();
        let ability = "project_list";
        let issuer = "easynet:///r/example/authority";
        let callee = "easynet:///r/example/service/alice.pages";
        let subject = crate::core::ura::resource_dot_ura(
            "example",
            "service.alice.pages",
            "read/project_list",
        );
        let signing_key = SigningKey::from_bytes(&[0x58; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Read);
        request.session_id = "realm-runtime-invocation-adapter-pages-list".to_string();
        request.callee_ura = callee.to_string();
        request.audience = callee.to_string();
        request.subject_ura = subject.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Read,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("Service callee RealmRuntimeInvocationAdapter authority must verify")
        .expect("session authority must project");

        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn realm_runtime_invocation_adapter_admits_publishing_agent_lifecycle_subject() {
        let now_ms = current_unix_ms();
        let ability = "browser.open_session";
        let issuer = "easynet:///r/example/authority";
        let callee = "easynet:///r/example/agent/device.node-a.browser";
        let signing_key = SigningKey::from_bytes(&[0x5c; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Invoke);
        request.session_id = "realm-runtime-invocation-adapter-browser-open".to_string();
        request.callee_ura = callee.to_string();
        request.audience = callee.to_string();
        request.subject_ura = callee.to_string();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(callee));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Invoke,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("publishing Agent lifecycle subject must verify")
        .expect("session authority must project");
    }

    #[test]
    fn realm_runtime_invocation_adapter_admits_device_owned_lifecycle_resource() {
        let now_ms = current_unix_ms();
        let ability = "browser.close_session";
        let issuer = "easynet:///r/example/authority";
        let callee = "easynet:///r/example/agent/device.node-a.browser";
        let subject =
            crate::core::ura::resource_dot_ura("example", "device.node-a", "browser/session-1");
        let signing_key = SigningKey::from_bytes(&[0x5d; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Manage);
        request.session_id = "realm-runtime-invocation-adapter-browser-close".to_string();
        request.callee_ura = callee.to_string();
        request.audience = callee.to_string();
        request.subject_ura = subject.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Manage,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("Device-owned lifecycle Resource subject must verify")
        .expect("session authority must project");
    }

    #[test]
    fn realm_runtime_invocation_adapter_rejects_non_agent_callee() {
        let now_ms = current_unix_ms();
        let ability = "fs.read";
        let issuer = "easynet:///r/example/authority";
        let device = "easynet:///r/example/device/node-a";
        let signing_key = SigningKey::from_bytes(&[0x57; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Read);
        request.session_id = "realm-runtime-invocation-adapter-1".to_string();
        request.callee_ura = device.to_string();
        request.audience = device.to_string();

        let error = authority_metadata::CanonicalSessionAuthorityIssuer::issue(
            request,
            issuer,
            |canonical| {
                Ok::<_, std::convert::Infallible>(signing_key.sign(canonical).to_bytes().to_vec())
            },
        )
        .expect_err("runtime adapter must never issue authority to a Device callee");

        assert_eq!(error.reason(), REASON_AUTHORITY_FORMAT_INVALID);
        assert!(error.to_string().contains("callee_ura"), "{error}");
    }

    #[test]
    fn realm_peer_runtime_invocation_adapter_admits_exact_foreign_agent_tuple() {
        let now_ms = current_unix_ms();
        let ability = "fs.read";
        let issuer = "easynet:///r/example/authority";
        let callee = "easynet:///r/peer.example/agent/device.node-a.runtime";
        let signing_key = SigningKey::from_bytes(&[0x58; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Read);
        request.session_id = "realm-peer-runtime-invocation-adapter-1".to_string();
        request.callee_ura = callee.to_string();
        request.audience = callee.to_string();
        let subject = request.subject_ura.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Read,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("exact peer-runtime adapter authority must verify")
        .expect("session authority must project");

        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn destination_admission_verifies_peer_runtime_authority_from_hub_attested_key() {
        let now_ms = current_unix_ms();
        let ability = "shell.run";
        let issuer = "easynet:///r/example/authority";
        let callee = "easynet:///r/peer.example/agent/device.node-a.locomotion";
        let signing_key = SigningKey::from_bytes(&[0x6a; 32]);
        let encoded_key = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let resolver = FederatedKeyResolver::new(
            SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default())),
            None,
            SharedFederatedPeers::default(),
            Some("peer.example".to_string()),
        );
        resolver
            .hub_attested_caller_keys()
            .attest_external_caller_key(issuer, &encoded_key, std::slice::from_ref(&encoded_key))
            .expect("authenticated upstream Hub attests the origin Authority key");
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Invoke);
        request.session_id = "realm-peer-runtime-invocation-adapter-shell".to_string();
        request.callee_ura = callee.to_string();
        request.audience = callee.to_string();
        let subject = request.subject_ura.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(&subject));

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            AccessAction::Invoke,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &|issuer_ura| {
                resolver
                    .resolve_all(issuer_ura)
                    .map(|keys| {
                        keys.into_iter()
                            .map(|key| BASE64_STANDARD.encode(key.to_bytes()))
                            .collect()
                    })
                    .map_err(axon_error_to_status)
            },
        )
        .expect("peer runtime authority must pass destination admission")
        .expect("session authority must project");

        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn realm_peer_runtime_invocation_adapter_rejects_local_agent_callee() {
        let now_ms = current_unix_ms();
        let ability = "fs.read";
        let issuer = "easynet:///r/example/authority";
        let local_callee = "easynet:///r/example/agent/device.node-a.runtime";
        let signing_key = SigningKey::from_bytes(&[0x59; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, ability, AccessAction::Read);
        request.session_id = "realm-peer-runtime-invocation-adapter-1".to_string();
        request.callee_ura = local_callee.to_string();
        request.audience = local_callee.to_string();
        let subject = request.subject_ura.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(local_callee), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let error = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &envelope,
                ability,
                AccessAction::Read,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "peer runtime adapter must reject a same-realm callee",
        );
        assert_eq!(error.code(), Code::PermissionDenied);
    }

    #[test]
    fn typed_governance_adapters_admit_only_their_exact_runtime_shapes() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/authority";
        let agent = "easynet:///r/example/agent/device.node-a.runtime-introspection";
        let signing_key = SigningKey::from_bytes(&[0x5a; 32]);
        let cases = [
            (
                "authority",
                "authority-governance-adapter-1",
                "federation.revoke",
                AccessAction::Manage,
                issuer,
                false,
            ),
            (
                "agent",
                "agent-governance-adapter-1",
                "meta.list_abilities",
                AccessAction::Read,
                agent,
                true,
            ),
            (
                "receipt",
                "realm-receipt-history-adapter-1",
                "invocation.history.list",
                AccessAction::Read,
                agent,
                true,
            ),
        ];

        for (label, session_id, ability, action, callee, runtime_state_subject) in cases {
            let mut request = realm_account_adapter_session_request(now_ms, ability, action);
            request.session_id = session_id.to_string();
            request.callee_ura = callee.to_string();
            request.audience = callee.to_string();
            if runtime_state_subject {
                request.subject_ura = crate::core::ura::resource_dot_ura(
                    "example",
                    "user.alice",
                    "runtime-state/read",
                );
            }
            let subject = request.subject_ura.clone();
            let metadata = signed_session_metadata(request, &signing_key);
            let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(&subject));
            let resolver = issuer_key_resolver(issuer, &signing_key);

            let authority = verify_authority_metadata_with_issuer_key(
                &envelope,
                ability,
                action,
                Some(&metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            )
            .unwrap_or_else(|error| panic!("{label} governance adapter rejected: {error}"))
            .expect("governance session authority must project");
            assert_eq!(authority.binding.form(), "session_of+session");
        }
    }

    #[test]
    fn authority_governance_adapter_binds_directory_stream_to_exact_authority_subject() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/authority";
        let ability = "federation.subscribe_directory_v2";
        let action = AccessAction::Stream;
        let subject = crate::core::ura::resource_dot_ura(
            "example",
            "authority",
            "invoke/federation.subscribe_directory_v2",
        );
        let signing_key = SigningKey::from_bytes(&[0x5c; 32]);
        let mut request = realm_account_adapter_session_request(now_ms, ability, action);
        request.session_id = "authority-governance-adapter-directory-stream".to_string();
        request.session_owner_user_id = "backend-authority".to_string();
        request.callee_ura = issuer.to_string();
        request.audience = issuer.to_string();
        request.subject_ura = subject.clone();
        let metadata = signed_session_metadata(request.clone(), &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(issuer), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            ability,
            action,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("exact Authority-owned directory subject must verify")
        .expect("directory stream session authority must project");
        assert_eq!(authority.binding.form(), "session_of+session");

        request.subject_ura = crate::core::ura::resource_dot_ura(
            "example",
            "authority",
            "invoke/federation.discover",
        );
        let wrong_subject_metadata = signed_session_metadata(request, &signing_key);
        let error = require_authority_metadata_error(
            verify_authority_metadata_with_issuer_key(
                &authority_wire_envelope(
                    Some(issuer),
                    Some(issuer),
                    Some("easynet:///r/example/resource/authority/invoke/federation.discover"),
                ),
                ability,
                action,
                Some(&wrong_subject_metadata),
                now_ms,
                RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                &resolver,
            ),
            "AuthorityGovernance proof must not cross-authorize another descriptor subject",
        );
        assert_eq!(error.code(), Code::PermissionDenied);
        assert!(
            error
                .message()
                .contains("must exactly bind Authority ability"),
            "{error}"
        );
    }

    #[test]
    fn lifecycle_session_adapter_binds_authority_to_exact_session_resource() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/authority";
        let callee = "easynet:///r/example/agent/device.node-a.runtime";
        let subject = crate::core::ura::resource_dot_ura(
            "example",
            "user.alice",
            "session/terminal-session-1",
        );
        let signing_key = SigningKey::from_bytes(&[0x5b; 32]);
        let mut request =
            realm_account_adapter_session_request(now_ms, "terminal.attach", AccessAction::Invoke);
        request.session_id = "terminal-session-1".to_string();
        request.callee_ura = callee.to_string();
        request.audience = callee.to_string();
        request.subject_ura = subject.clone();
        let metadata = signed_session_metadata(request, &signing_key);
        let envelope = authority_wire_envelope(Some(issuer), Some(callee), Some(&subject));
        let resolver = issuer_key_resolver(issuer, &signing_key);

        let authority = verify_authority_metadata_with_issuer_key(
            &envelope,
            "terminal.attach",
            AccessAction::Invoke,
            Some(&metadata),
            now_ms,
            RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
            &resolver,
        )
        .expect("exact lifecycle session authority must verify")
        .expect("lifecycle session authority must project");
        assert_eq!(authority.binding.form(), "session_of+session");
    }

    #[test]
    fn realm_account_adapter_profiles_do_not_cross_authorize() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/authority";
        let signing_key = SigningKey::from_bytes(&[0x54; 32]);

        for (ability, action, wrong_session_id) in [
            (
                ABILITY_IDENTITY_REGISTER_PUBKEY,
                AccessAction::Manage,
                "realm-account-adapter-wrong-profile",
            ),
            (
                governance::PRINCIPAL_GET,
                AccessAction::Read,
                "realm-identity-adapter-wrong-profile",
            ),
            (
                federation::NAMESPACE_RESOLVE,
                AccessAction::Read,
                "realm-identity-adapter-wrong-profile",
            ),
            (
                ABILITY_IDENTITY_REGISTER_PUBKEY,
                AccessAction::Manage,
                "realm-directory-read-adapter-wrong-profile",
            ),
            (
                ABILITY_IDENTITY_REGISTER_PUBKEY,
                AccessAction::Manage,
                "realm-runtime-invocation-adapter-wrong-profile",
            ),
            (
                "fs.read",
                AccessAction::Read,
                "realm-account-adapter-wrong-profile",
            ),
        ] {
            let mut request = realm_account_adapter_session_request(now_ms, ability, action);
            request.session_id = wrong_session_id.to_string();
            let subject = request.subject_ura.clone();
            let metadata = signed_session_metadata(request, &signing_key);
            let envelope = authority_wire_envelope(Some(issuer), Some(issuer), Some(&subject));
            let resolver = issuer_key_resolver(issuer, &signing_key);

            let error = require_authority_metadata_error(
                verify_authority_metadata_with_issuer_key(
                    &envelope,
                    ability,
                    action,
                    Some(&metadata),
                    now_ms,
                    RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                    &resolver,
                ),
                "typed adapter profiles must not cross-authorize",
            );
            assert_eq!(error.code(), Code::PermissionDenied, "{error}");
            assert!(
                error.message().contains(REASON_AUTHORITY_ISSUER_DENIED),
                "{error}"
            );
        }
    }

    #[test]
    fn realm_account_adapter_session_rejects_non_exact_or_non_principal_shapes() {
        let now_ms = current_unix_ms();
        let issuer = "easynet:///r/example/authority";
        let signing_key = SigningKey::from_bytes(&[0x52; 32]);

        for case in [
            "session_id",
            "creator",
            "audience",
            "callee",
            "caller",
            "caller_realm",
            "callee_realm",
            "issuer_realm",
            "subject_owner",
            "ability",
            "scope",
            "followup",
            "actions",
            "descriptor_action",
            "ttl",
        ] {
            let mut ability = governance::PRINCIPAL_GET.to_string();
            let mut action = AccessAction::Read;
            let mut request =
                realm_account_adapter_session_request(now_ms, governance::PRINCIPAL_GET, action);
            let mut caller = issuer.to_string();
            match case {
                "session_id" => request.session_id = "ordinary-session".into(),
                "creator" => {
                    request.creator_principal_id = "easynet:///r/example/user/alice".into()
                }
                "audience" => {
                    request.audience = "easynet:///r/example/agent/backend.adapter".into()
                }
                "callee" => {
                    request.callee_ura = "easynet:///r/example/agent/backend.adapter".into();
                    request.audience = request.callee_ura.clone();
                }
                "caller" => {
                    caller = "easynet:///r/example/agent/backend.adapter".into();
                }
                "caller_realm" => {
                    caller = "easynet:///r/other/authority".into();
                }
                "callee_realm" => {
                    request.callee_ura = "easynet:///r/other/authority".into();
                    request.audience = request.callee_ura.clone();
                }
                "issuer_realm" => {
                    request.issuer_ura = "easynet:///r/other/authority".into();
                    request.creator_principal_id = request.issuer_ura.clone();
                    request.callee_ura = request.issuer_ura.clone();
                    request.audience = request.issuer_ura.clone();
                    caller = request.issuer_ura.clone();
                }
                "subject_owner" => request.session_owner_user_id = "bob".into(),
                "ability" => {
                    ability = "namespace.resolve".into();
                    request.subject_ura =
                        "easynet:///r/example/resource/user.alice/invoke/namespace.resolve".into();
                    request.scopes = vec![ability.clone()];
                    request.allowed_followup_abilities = vec![ability.clone()];
                }
                "scope" => request.scopes = vec!["principal.lifecycle.*".into()],
                "followup" => {
                    request.allowed_followup_abilities = vec!["principal.lifecycle.*".into()]
                }
                "actions" => request.allowed_actions = vec!["read".into(), "manage".into()],
                "descriptor_action" => {
                    request.allowed_actions = vec!["manage".into()];
                    action = AccessAction::Manage;
                }
                "ttl" => request.expires_at_ms = request.issued_at_ms + 5 * 60 * 1_000 + 1,
                _ => unreachable!(),
            }
            let subject = request.subject_ura.clone();
            let callee = request.callee_ura.clone();
            let request_issuer = request.issuer_ura.clone();
            let metadata = signed_session_metadata_unchecked(request, &signing_key);
            let envelope = authority_wire_envelope(Some(&caller), Some(&callee), Some(&subject));
            let resolver = issuer_key_resolver(&request_issuer, &signing_key);
            let err = require_authority_metadata_error(
                verify_authority_metadata_with_issuer_key(
                    &envelope,
                    &ability,
                    action,
                    Some(&metadata),
                    now_ms,
                    RuntimeAuthorityIssuerPolicy::RealmTrustAnchor,
                    &resolver,
                ),
                case,
            );
            let expected_code = if matches!(case, "followup" | "subject_owner") {
                Code::InvalidArgument
            } else {
                Code::PermissionDenied
            };
            assert_eq!(err.code(), expected_code, "case={case}: {err}");
        }
    }

    #[test]
    fn authority_proof_resolver_authorizes_canonical_user_owner_ura() {
        let owner_ura = crate::core::ura::user_ura("example", "alice");
        let signing_key = SigningKey::from_bytes(&[0x45; 32]);
        let anchor =
            RealmTrustAnchor::from_entries(vec![crate::daemon::trust::anchor::TrustedAgent {
                agent_ura: owner_ura.clone(),
                public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            }])
            .expect("user trust anchor");
        let stores = AccessControlStoreRegistry::ephemeral();

        stores
            .with_store(&owner_ura, |store| {
                let resolver = StoreBackedAuthorityProofResolver {
                    trust_anchor: &anchor,
                    store,
                    now: Utc::now(),
                };
                assert!(resolver.issuer_authorized_for_owner_ura(&owner_ura, &owner_ura));
                assert!(!resolver.issuer_authorized_for_owner_ura(
                    &owner_ura,
                    &crate::core::ura::user_ura("other", "alice")
                ));
            })
            .expect("ephemeral owner store");
    }

    fn raw_authority_metadata(value: serde_json::Value) -> String {
        BASE64_STANDARD.encode(serde_json::to_vec(&value).expect("authority metadata JSON"))
    }

    fn assert_raw_authority_wire_error(error: Status, missing_field: &str) {
        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(
            error.message().contains(REASON_AUTHORITY_FORMAT_INVALID)
                && error.message().contains("JSON parse failed")
                && error
                    .message()
                    .contains(&format!("missing field `{missing_field}`")),
            "authority raw wire error must fail at strict JSON shape: {error}"
        );
        assert!(
            !error.message().contains("payload parse failed")
                && !error.message().contains("signature base64 decode failed"),
            "missing raw fields must not be reinterpreted as payload/signature defaults: {error}"
        );
    }

    fn assert_raw_authority_wire_unknown_field_error(error: Status, unknown_field: &str) {
        assert_eq!(error.code(), Code::InvalidArgument);
        assert!(
            error.message().contains(REASON_AUTHORITY_FORMAT_INVALID)
                && error.message().contains("JSON parse failed")
                && error
                    .message()
                    .contains(&format!("unknown field `{unknown_field}`")),
            "authority raw wire error must reject unknown fields at wire decode: {error}"
        );
        assert!(
            !error.message().contains("payload parse failed")
                && !error.message().contains("signature base64 decode failed"),
            "unknown raw fields must not be reinterpreted as payload/signature defaults: {error}"
        );
    }

    fn require_delegation_parse_error(
        raw: &str,
        trust_anchor: &RealmTrustAnchor,
        now_ms: i64,
    ) -> Status {
        match parse_and_verify_delegation_proof(raw, trust_anchor, now_ms) {
            Ok(_) => panic!("delegation authority raw wire unexpectedly parsed"),
            Err(error) => error,
        }
    }

    fn require_session_parse_error(
        raw: &str,
        trust_anchor: &RealmTrustAnchor,
        now_ms: i64,
    ) -> Status {
        match parse_and_verify_session_authority(raw, trust_anchor, now_ms) {
            Ok(_) => panic!("session authority raw wire unexpectedly parsed"),
            Err(error) => error,
        }
    }

    #[test]
    fn unknown_caller_status_uses_canonical_key_not_found_reason() {
        let err = permission_denied_unknown_caller("easynet:///r/test/user/missing");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message()
                .starts_with(SignatureDecisionReason::CallerKeyNotFound.as_str()),
            "{}",
            err.message()
        );
        assert!(
            !err.message().contains("CALLER_UNKNOWN"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn admission_authority_raw_wire_requires_payload_and_signature() {
        let trust_anchor = RealmTrustAnchor::default();
        let now_ms = 1_750_000_000_000;

        let err = require_delegation_parse_error(
            &raw_authority_metadata(json!({ "signature": "AA==" })),
            &trust_anchor,
            now_ms,
        );
        assert_raw_authority_wire_error(err, "payload");

        let err = require_delegation_parse_error(
            &raw_authority_metadata(json!({ "payload": {} })),
            &trust_anchor,
            now_ms,
        );
        assert_raw_authority_wire_error(err, "signature");

        let err = require_session_parse_error(
            &raw_authority_metadata(json!({ "signature": "AA==" })),
            &trust_anchor,
            now_ms,
        );
        assert_raw_authority_wire_error(err, "payload");

        let err = require_session_parse_error(
            &raw_authority_metadata(json!({ "payload": {} })),
            &trust_anchor,
            now_ms,
        );
        assert_raw_authority_wire_error(err, "signature");

        let err = require_delegation_parse_error(
            &raw_authority_metadata(json!({
                "payload": {},
                "signature": "AA==",
                "retired_signature_carrier": "compat"
            })),
            &trust_anchor,
            now_ms,
        );
        assert_raw_authority_wire_unknown_field_error(err, "retired_signature_carrier");

        let err = require_session_parse_error(
            &raw_authority_metadata(json!({
                "payload": {},
                "signature": "AA==",
                "retired_signature_carrier": "compat"
            })),
            &trust_anchor,
            now_ms,
        );
        assert_raw_authority_wire_unknown_field_error(err, "retired_signature_carrier");
    }

    fn assert_complete_non_self_policy(
        authority: VerifiedRuntimeAuthority,
        envelope: &DescriptorBoundEnvelope,
        expected_form: &str,
    ) {
        let decision = receipt_policy_decision(
            envelope.envelope().caller.ura.as_str(),
            envelope.envelope().callee.ura.as_str(),
            envelope.envelope().subject.ura.as_str(),
        );
        let authority = authority
            .with_policy_decision(&decision)
            .expect("policy decision must bind into proof payload");
        assert_eq!(authority.binding.form(), expected_form);
        assert_ne!(authority.binding.form(), "self+identity");
        let proof = authority.authority_proof(envelope);
        assert_eq!(proof.binding.as_ref(), Some(&authority.binding));
        assert!(
            !proof.proof_payload.is_empty(),
            "successful authority proof must carry daemon admission proof payload"
        );
        let proof_payload: serde_json::Value =
            serde_json::from_slice(&proof.proof_payload).expect("canonical proof payload JSON");
        assert_eq!(
            proof_payload["profile"],
            "easynet-runtime-admission-proof-v1"
        );
        assert_eq!(proof_payload["policy_decision"]["decision"], "allow");
        assert_eq!(
            proof_payload["policy_decision"]["reason"],
            "EXPLICIT_GRANT_ALLOW"
        );
        assert!(proof_payload["policy_decision_hash"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:")));
        assert_ne!(proof.proof_hash, [0u8; 32]);
        assert_eq!(proof.proof_hash, authority_proof_expected_hash(&proof));
        proof
            .validate_complete()
            .expect("verified product authority must produce complete receipt proof facts");
        authority
            .into_policy(envelope)
            .expect("complete product authority must construct canonical admission policy");
    }

    fn receipt_policy_decision(
        caller_ura: &str,
        callee_ura: &str,
        subject_ura: &str,
    ) -> PolicyDecision {
        PolicyDecision {
            decision: crate::daemon::invocation::admission::decision::PolicyDecisionOutcome::Allow,
            reason: crate::daemon::invocation::admission::decision::PolicyDecisionReason::ExplicitGrantAllow,
            owner_user_ura: Some("easynet:///r/policy/user/alice".to_string()),
            owner_source: crate::daemon::invocation::admission::decision::OwnerSource::Subject,
            caller_ura: caller_ura.to_string(),
            principal_kind: crate::daemon::invocation::admission::decision::PrincipalKind::Agent,
            principal_id: caller_ura.to_string(),
            token_id: None,
            token_class: None,
            callee_ura: callee_ura.to_string(),
            subject_ura: subject_ura.to_string(),
            ability_ura: "easynet:///r/policy/ability/agent.service.worker.run".to_string(),
            action: AccessAction::Invoke,
            rejector_ura: Some(callee_ura.to_string()),
            policy_rule_id: Some("grant-1".to_string()),
            grant_id: Some("grant-1".to_string()),
            prompt_request_id: None,
            canonical_hash: Some("sha256:admission".to_string()),
            signature_key_id: Some("ed25519:test".to_string()),
            authority_proof_id: Some("authority-proof-1".to_string()),
        }
    }

    #[test]
    fn delegated_admission_produces_complete_non_self_receipt_policy() {
        let caller_ura = "easynet:///r/policy/agent/alice.delegate";
        let callee_ura = "easynet:///r/policy/agent/service.worker";
        let subject_ura = "easynet:///r/policy/resource/user.bob/document/report";
        let payload = DelegationPayload {
            issuer_ura: "easynet:///r/policy/user/alice".to_string(),
            subject_ura: subject_ura.to_string(),
            caller_ura: caller_ura.to_string(),
            audience: callee_ura.to_string(),
            scopes: vec!["run".to_string()],
            issued_at_ms: 1_700_000_000_000,
            expires_at_ms: 1_800_000_000_000,
        };
        let canonical_payload =
            authority_metadata::canonical_authority_payload_bytes(&payload).expect("payload");
        let authority = VerifiedRuntimeAuthority::delegated(VerifiedSignedAuthority {
            payload,
            canonical_payload,
            signature: vec![0x31; ed25519_dalek::SIGNATURE_LENGTH],
        })
        .expect("verified delegation must project to generic authority");
        let envelope = receipt_policy_envelope(caller_ura, callee_ura, subject_ura);

        assert_complete_non_self_policy(authority, &envelope, "delegated_by+delegation");
    }

    #[test]
    fn session_admission_produces_complete_non_self_receipt_policy() {
        let caller_ura = "easynet:///r/policy/authority";
        let callee_ura = "easynet:///r/policy/agent/service.worker";
        let subject_ura = "easynet:///r/policy/resource/user.alice/session/session-42";
        let payload = SessionAuthorityPayload {
            issuer_ura: caller_ura.to_string(),
            session_id: "session-42".to_string(),
            session_owner_user_id: "alice".to_string(),
            creator_principal_id: "backend".to_string(),
            callee_ura: callee_ura.to_string(),
            subject_ura: subject_ura.to_string(),
            audience: callee_ura.to_string(),
            scopes: vec!["run".to_string()],
            allowed_actions: vec!["invoke".to_string()],
            allowed_followup_abilities: vec!["run".to_string()],
            issued_at_ms: 1_700_000_000_000,
            expires_at_ms: 1_800_000_000_000,
        };
        let canonical_payload =
            authority_metadata::canonical_authority_payload_bytes(&payload).expect("payload");
        let authority = VerifiedRuntimeAuthority::session(VerifiedSignedAuthority {
            payload,
            canonical_payload,
            signature: vec![0x52; ed25519_dalek::SIGNATURE_LENGTH],
        })
        .expect("verified session must project to generic authority");
        let envelope = receipt_policy_envelope(caller_ura, callee_ura, subject_ura);

        assert_complete_non_self_policy(authority, &envelope, "session_of+session");
    }

    #[test]
    fn trusted_local_system_capability_receipt_is_explicit_and_hash_bound() {
        let caller_ura = "easynet:///r/policy/authority";
        let callee_ura = "easynet:///r/policy/agent/service.worker";
        let subject_ura = "easynet:///r/policy/resource/user.alice/session/session-42";
        let envelope = receipt_policy_envelope(caller_ura, callee_ura, subject_ura);
        let authority =
            VerifiedRuntimeAuthority::trusted_local_system_capability(envelope.envelope())
                .expect("trusted local system authority must construct from a canonical callee URA")
                .with_runtime_admission_fact("trusted-local-system capability admission")
                .expect("runtime admission fact must bind into proof payload");
        let proof = authority.authority_proof(&envelope);
        let proof_payload: serde_json::Value =
            serde_json::from_slice(&proof.proof_payload).expect("canonical proof payload JSON");

        assert_eq!(
            proof_payload["profile"],
            "easynet-runtime-admission-proof-v1"
        );
        assert_eq!(
            proof_payload["bootstrap_admission"]["reason"],
            "trusted-local-system capability admission"
        );
        // trusted_local_system_capability is a daemon-internal admission
        // fact (the daemon vouches for its own structural policy
        // evaluation, not a caller-presented cryptographic claim) — it
        // maps to Bootstrap, not a signed AuthorityBinding relation. See
        // RFC 001-authority-binding-relation-evidence.md.
        assert_eq!(authority.binding.form(), "bootstrap");
        assert!(matches!(
            &authority.binding,
            AuthorityOrBootstrap::Bootstrap(bootstrap)
                if bootstrap.principal_ura == caller_ura
                    && bootstrap.realm == "policy"
                    && bootstrap.ability == envelope.envelope().ability
        ));
        assert_ne!(authority.binding.form(), "self+identity");
        assert!(proof_payload.get("policy_decision").is_none());
        assert_eq!(proof.proof_hash, authority_proof_expected_hash(&proof));
        proof
            .validate_complete()
            .expect("runtime admission fact must produce complete receipt proof facts");
    }

    #[test]
    fn session_authority_binding_requires_explicit_envelope_subject() {
        let caller_ura = "easynet:///r/policy/authority";
        let callee_ura = &crate::core::ura::device_agent_ura("policy", "dev-a", "worker");
        let payload = SessionAuthorityPayload {
            issuer_ura: caller_ura.to_string(),
            session_id: "session-42".to_string(),
            session_owner_user_id: "alice".to_string(),
            creator_principal_id: "backend".to_string(),
            callee_ura: callee_ura.to_string(),
            subject_ura: "easynet:///r/policy/resource/user.alice/session/session-42".to_string(),
            audience: callee_ura.to_string(),
            scopes: vec!["run".to_string()],
            allowed_actions: vec!["invoke".to_string()],
            allowed_followup_abilities: vec!["run".to_string()],
            issued_at_ms: 1_700_000_000_000,
            expires_at_ms: 1_800_000_000_000,
        };
        let envelope = authority_wire_envelope(Some(caller_ura), Some(callee_ura), None);
        let err =
            verify_session_authority_bindings(&payload, &envelope, "run", AccessAction::Invoke)
                .expect_err("missing envelope subject must fail as an incomplete tuple");

        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(
            err.message().contains(REASON_ENVELOPE_INCOMPLETE)
                && err.message().contains("envelope.subject.ura"),
            "unexpected error for missing subject: {err}"
        );
        assert!(
            !err.message().contains(REASON_AUTHORITY_SUBJECT_MISMATCH),
            "missing subject must not be reported as authority mismatch: {err}"
        );
    }

    #[test]
    fn authority_ability_projection_rejects_non_canonical_ability_ura() {
        let err = public_name_from_authority_ability_ura("easynet:///r/policy/device/dev-a")
            .expect_err("authority projection must not repair non-Ability URA");

        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(
            err.message().contains("non-canonical ability URA"),
            "unexpected error: {err}"
        );
        assert!(
            !err.message().contains("AUTHORITY_SCOPE_VIOLATION"),
            "identity projection failures must happen before scope matching: {err}"
        );
    }

    #[test]
    fn delegation_binding_requires_explicit_envelope_callee() {
        let caller_ura = "easynet:///r/policy/agent/alice.delegate";
        let subject_ura = "easynet:///r/policy/resource/user.bob/document/report";
        let payload = DelegationPayload {
            issuer_ura: "easynet:///r/policy/user/alice".to_string(),
            subject_ura: subject_ura.to_string(),
            caller_ura: caller_ura.to_string(),
            audience: "easynet:///r/policy/agent/service.worker".to_string(),
            scopes: vec!["run".to_string()],
            issued_at_ms: 1_700_000_000_000,
            expires_at_ms: 1_800_000_000_000,
        };
        let envelope = authority_wire_envelope(Some(caller_ura), None, Some(subject_ura));
        let err = verify_delegation_bindings(&payload, &envelope, "run")
            .expect_err("missing envelope callee must fail as an incomplete tuple");

        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(
            err.message().contains(REASON_ENVELOPE_INCOMPLETE)
                && err.message().contains("envelope.callee.ura"),
            "unexpected error for missing callee: {err}"
        );
        assert!(
            !err.message().contains(REASON_AUTHORITY_AUDIENCE_VIOLATION),
            "missing callee must not be reported as audience violation: {err}"
        );
    }

    #[test]
    fn raw_envelope_tuple_rejects_all_zero_principal_identities() {
        let placeholder = "00000000-0000-0000-0000-000000000000";
        let caller = "easynet:///r/policy/agent/alice.delegate";
        let callee = "easynet:///r/policy/agent/service.worker";
        let subject = "easynet:///r/policy/resource/user.alice/document/report";

        for (field, envelope, validate) in [
            (
                "caller",
                authority_wire_envelope(
                    Some(&crate::core::ura::user_ura("policy", placeholder)),
                    Some(callee),
                    Some(subject),
                ),
                caller_ura_required as fn(&Envelope) -> Result<&str, Status>,
            ),
            (
                "callee",
                authority_wire_envelope(
                    Some(caller),
                    Some(&crate::core::ura::user_ura("policy", placeholder)),
                    Some(subject),
                ),
                callee_ura_required as fn(&Envelope) -> Result<&str, Status>,
            ),
            (
                "subject",
                authority_wire_envelope(
                    Some(caller),
                    Some(callee),
                    Some(&format!(
                        "easynet:///r/policy/resource/user.{placeholder}/document/report"
                    )),
                ),
                subject_ura_required as fn(&Envelope) -> Result<&str, Status>,
            ),
        ] {
            let error = validate(&envelope)
                .expect_err("raw all-zero identity must fail before authority verification");
            assert_eq!(error.code(), Code::InvalidArgument);
            assert!(
                error.message().contains(REASON_ENVELOPE_INCOMPLETE)
                    && error.message().contains(&format!("envelope.{field}.ura"))
                    && error.message().contains("all-zero principal placeholder"),
                "wrong {field} error: {error}"
            );
        }
    }

    struct RejectingFederationClient;

    #[async_trait::async_trait]
    impl FederationClient for RejectingFederationClient {
        async fn invoke(
            &self,
            target_hub_endpoint: &HubEndpoint,
            _request: InvokeRequest,
        ) -> Result<InvokeResponse, FederationClientError> {
            Err(FederationClientError::DialFailed {
                endpoint: target_hub_endpoint.clone(),
                detail: "admission classification test does not perform network I/O".to_string(),
            })
        }
    }

    fn federated_facade() -> AdmissionFacade {
        let mut peers = BTreeMap::new();
        peers.insert(
            "peer-realm".to_string(),
            "https://peer-realm.example:50443".to_string(),
        );
        let trust = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let resolver = Arc::new(
            FederatedKeyResolver::new(
                trust.clone(),
                Some(Arc::new(RejectingFederationClient)),
                SharedFederatedPeers::new(peers),
                Some("self-realm".to_string()),
            )
            .with_hub_signer(Arc::new(
                crate::daemon::identity::self_identity::TestCanonicalSigner::new(
                    crate::core::ura::hub_ura("self-realm"),
                    [0x54; 32],
                ),
            )),
        );
        AdmissionFacade::with_trust_anchor_cell(
            trust,
            Some(crate::core::ura::hub_ura("self-realm")),
        )
        .with_federated_key_resolver(resolver)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receipt_key_resolver_reuses_federated_authority_for_cross_realm_signers() {
        let resolver = federated_facade().receipt_key_resolver();
        let peer_device = "easynet:///r/peer-realm/device/peer-device";

        let error = resolver
            .resolve(peer_device)
            .expect_err("peer-realm receipt signer should use federated resolution");

        assert_eq!(
            error.reason,
            axon_sdk::invocation::ErrorCode::CallerKeyNotFound.as_str()
        );
        assert!(
            error.message.contains("dial_failed")
                && error
                    .message
                    .contains("admission classification test does not perform network I/O"),
            "cross-realm receipt verifier must reach the federated provider, got {error:?}"
        );
        assert!(
            !error.message.contains("realm_trust_anchor")
                && !error.message.contains("no trust-anchor entry"),
            "cross-realm receipt verifier must not fall back to the local-only trust-anchor adapter: {error:?}"
        );
    }

    #[test]
    fn caller_role_resolves_same_realm_user_from_trust_anchor_user_bucket() {
        let user = "easynet:///r/self-realm/user/alice";
        let signing_key = SigningKey::from_bytes(&[0x61; 32]);
        let user_row = crate::daemon::trust::anchor::TrustedAgent {
            agent_ura: user.to_string(),
            public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
            role: TrustAnchorRole::User,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![user_row]).expect("user trust bucket anchor"),
        );
        let facade = AdmissionFacade::with_trust_anchor_cell(
            SharedTrustAnchor::new(Arc::clone(&anchor)),
            Some(crate::core::ura::hub_ura("self-realm")),
        );

        assert_eq!(
            facade
                .trusted_path_for_caller(user, anchor.as_ref(), "chat", None)
                .expect("same-realm trust-anchor User bucket should classify as User"),
            TrustedCallerPath::User
        );
    }

    #[test]
    fn device_policy_classifies_exact_live_hub_attested_user_without_peer_directory() {
        let user = "easynet:///r/origin.example/user/alice";
        let signing_key = SigningKey::from_bytes(&[0x63; 32]);
        let encoded_key = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let trust = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let resolver = Arc::new(FederatedKeyResolver::new(
            trust.clone(),
            None,
            SharedFederatedPeers::default(),
            Some("destination.example".to_string()),
        ));
        resolver
            .hub_attested_caller_keys()
            .attest_external_caller_key(user, &encoded_key, std::slice::from_ref(&encoded_key))
            .expect("authenticated upstream Hub attests the exact signed User key");
        let facade = AdmissionFacade::with_trust_anchor_cell(
            trust,
            Some(crate::core::ura::hub_ura("destination.example")),
        )
        .with_federated_key_resolver(resolver);

        assert_eq!(
            facade
                .trusted_path_for_caller(user, &RealmTrustAnchor::default(), "shell.run", None)
                .expect("Device policy must consume the live Hub attestation"),
            TrustedCallerPath::User
        );
    }

    #[test]
    fn trusted_device_row_requires_public_invocation_purpose() {
        let device = "easynet:///r/self-realm/device/dev-1";
        let signing_key = SigningKey::from_bytes(&[0x62; 32]);
        let device_row = crate::daemon::trust::anchor::TrustedAgent {
            agent_ura: device.to_string(),
            public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![device_row]).expect("device trust bucket anchor"),
        );
        let facade = AdmissionFacade::with_trust_anchor_cell(
            SharedTrustAnchor::new(Arc::clone(&anchor)),
            Some(crate::core::ura::hub_ura("self-realm")),
        );

        let authority = "easynet:///r/self-realm/authority";
        let ability = crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES;
        let purpose = verify_device_invocation_purpose(DeviceInvocationPurposeScope {
            caller_ura: device,
            callee_ura: authority,
            subject_ura: device,
            public_ability: ability,
            daemon_ura: Some(authority),
            action: AccessAction::Manage,
        })
        .expect("verified purpose");
        let path = facade
            .trusted_path_for_caller(device, anchor.as_ref(), ability, Some(purpose))
            .expect("verified Device custody must carry an admitted publication purpose");
        assert!(matches!(path, TrustedCallerPath::DeviceCustody(got) if got == purpose));
        let error = facade
            .trusted_path_for_caller(device, anchor.as_ref(), "shell.run", None)
            .expect_err("ordinary abilities must not classify a Device as an actor");
        assert_eq!(error.code(), tonic::Code::PermissionDenied);
        assert!(error.message().contains("DEVICE_CALLER_PURPOSE_UNVERIFIED"));
    }

    #[test]
    fn caller_role_resolves_active_same_realm_user_from_principal_lifecycle() {
        let dir = tempdir().expect("tempdir");
        let user = "easynet:///r/self-realm/user/alice";
        let mut principals = serde_json::Map::new();
        principals.insert(
            user.to_string(),
            json!({
                "principal_ura": user,
                "state": "active",
                "version": 2,
                "bindings": [],
                "enrollment_proof": {
                    "kind": "bootstrap",
                    "reference": "proof:create"
                },
                "consumed_recovery_proofs": {},
                "enrollments": [],
                "grants": [],
                "created_unix_ms": 1,
                "updated_unix_ms": 1,
                "command_log": {"create": 1}
            }),
        );
        let store_path = dir.path().join("principal-lifecycle.json");
        std::fs::write(
            &store_path,
            serde_json::to_vec(&json!({ "principals": principals })).expect("store json"),
        )
        .expect("write lifecycle store");
        let facade = AdmissionFacade::with_trust_anchor_cell(
            SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default())),
            Some(crate::core::ura::hub_ura("self-realm")),
        )
        .with_principal_lifecycle_reader(PrincipalLifecycleReader::new(store_path));

        assert_eq!(
            facade
                .trusted_path_for_caller(user, &RealmTrustAnchor::default(), "chat", None)
                .expect("active lifecycle user should be trusted as User"),
            TrustedCallerPath::User
        );
    }

    #[test]
    fn off_box_facade_does_not_accept_daemon_ura_spoof_as_local_self() {
        assert!(
            !AdmissionTransportBoundary::OffBoxStrict.accepts_local_self_caller(
                Some("easynet:///r/test/device/daemon"),
                "easynet:///r/test/device/daemon",
            )
        );
    }

    #[test]
    fn admission_facade_defaults_to_off_box_strict() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);

        assert_eq!(
            facade.transport_boundary(),
            AdmissionTransportBoundary::OffBoxStrict
        );
        assert!(!facade.transport_boundary().accepts_local_self_caller(
            Some("easynet:///r/test/authority"),
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
        ));
    }

    #[test]
    fn authenticated_local_ipc_must_opt_in_to_local_system_admission() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None)
            .with_transport_boundary(AdmissionTransportBoundary::LocalOnlyIpc);

        assert!(facade.transport_boundary().accepts_local_self_caller(
            None,
            crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
        ));
    }

    #[test]
    fn authority_proof_admission_requires_daemon_audience() {
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None);

        let error = facade
            .authority_proof_audience_ura()
            .expect_err("authority proof admission must not infer audience from callee");

        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        assert!(
            error.message().contains("AUTHORITY_PROOF_AUDIENCE_MISSING"),
            "unexpected error: {error}"
        );
        assert!(
            error.message().contains("refusing to infer proof audience"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn off_box_facade_does_not_accept_local_system_self_admission() {
        assert!(
            !AdmissionTransportBoundary::OffBoxStrict.accepts_local_self_caller(
                Some("easynet:///r/test/device/daemon"),
                crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            )
        );
    }

    #[test]
    fn register_pubkey_bootstrap_authority_is_selected_only_for_user_self_registration() {
        let authority = crate::core::ura::hub_ura("test");
        let ability_subject =
            crate::core::ura::owner_ability_ura(&authority, ABILITY_IDENTITY_REGISTER_PUBKEY)
                .expect("identity register ability subject");
        let user = crate::core::ura::user_ura("test", "alice");
        let user_bootstrap =
            authority_wire_envelope(Some(&user), Some(&authority), Some(&ability_subject));
        assert!(uses_bootstrap_authority(
            &user_bootstrap,
            ABILITY_IDENTITY_REGISTER_PUBKEY
        ));

        let authority_device_registration = authority_wire_envelope(
            Some(&authority),
            Some(&authority),
            Some(&format!(
                "easynet:///r/test/resource/user.alice/invoke/{ABILITY_IDENTITY_REGISTER_PUBKEY}"
            )),
        );
        assert!(
            !uses_bootstrap_authority(
                &authority_device_registration,
                ABILITY_IDENTITY_REGISTER_PUBKEY
            ),
            "realm-authority device registration must verify normal caller-signed authority metadata"
        );
    }

    #[test]
    fn federated_caller_classification_accepts_only_canonical_peer_identities() {
        let facade = federated_facade();

        assert!(facade.is_federated_caller("easynet:///r/peer-realm/authority"));
        assert!(facade.is_federated_caller("easynet:///r/peer-realm/agent/alice.worker"));
        assert!(!facade.is_federated_caller("easynet:///r/peer-realm/authority/extra"));
        assert!(!facade.is_federated_caller("easynet:///r/self-realm/authority"));
        assert!(!facade.is_federated_caller("easynet:///r/unknown-realm/authority"));
    }

    #[test]
    fn federated_agent_caller_projects_as_agent_principal() {
        let facade = federated_facade();
        let caller = "easynet:///r/peer-realm/agent/alice.worker";
        let path = facade
            .trusted_path_for_caller(caller, &RealmTrustAnchor::default(), "chat", None)
            .expect("peer-realm Agent caller must classify through federated trust path");

        assert_eq!(path, TrustedCallerPath::AgentDeviceCustody);

        let principal = principal_for(path, caller, &RealmTrustAnchor::default())
            .expect("Agent URA must project as Agent principal");
        assert_eq!(
            principal.kind,
            crate::daemon::invocation::admission::decision::PrincipalKind::Agent
        );
        assert_eq!(principal.id, caller);
        assert!(principal.token_id.is_none());
        assert!(principal.caller_user_ura.is_none());
    }

    #[test]
    fn local_hosted_agent_key_fallback_projects_direct_agent_custody_path() {
        let signing_key = SigningKey::from_bytes(&[0x6b; 32]);
        let agent_ura = crate::core::ura::agent_ura("admission-test", "alice", "testbot");
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None)
            .with_invocation_verification_keys(Arc::new(StaticInvocationVerificationKeyProvider {
                caller_ura: agent_ura.clone(),
                verifying_key: signing_key.verifying_key(),
            }));

        let path = facade
            .trusted_path_for_caller(&agent_ura, &RealmTrustAnchor::default(), "chat", None)
            .expect("hosted Agent key fallback must classify Agent caller");
        assert_eq!(path, TrustedCallerPath::AgentDeviceCustody);

        let principal = principal_for(path, &agent_ura, &RealmTrustAnchor::default())
            .expect("Agent custody path must project Agent principal");
        assert_eq!(
            principal.kind,
            crate::daemon::invocation::admission::decision::PrincipalKind::Agent
        );
    }

    #[test]
    fn local_hosted_agent_key_fallback_rejects_device_caller_ura() {
        let signing_key = SigningKey::from_bytes(&[0x6c; 32]);
        let device_ura = crate::core::ura::device_ura("admission-test", "device-a");
        let facade = AdmissionFacade::new(Arc::new(RealmTrustAnchor::default()), None)
            .with_invocation_verification_keys(Arc::new(StaticInvocationVerificationKeyProvider {
                caller_ura: device_ura.clone(),
                verifying_key: signing_key.verifying_key(),
            }));

        let error = facade
            .trusted_path_for_caller(&device_ura, &RealmTrustAnchor::default(), "chat", None)
            .expect_err("local hosted-Agent key fallback must not classify Device callers");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(
            error
                .message()
                .contains("LOCAL_HOSTED_AGENT_CALLER_KIND_MISMATCH"),
            "{error:?}"
        );
    }

    #[test]
    fn admission_realm_extraction_uses_the_canonical_ura_parser() {
        assert_eq!(
            crate::core::ura::realm_from_ura("easynet:///r/peer-realm/authority"),
            Some("peer-realm".to_string())
        );
        assert_eq!(
            crate::core::ura::realm_from_ura("easynet:///r/peer-realm/authority/extra"),
            None
        );
    }

    #[test]
    fn agent_descriptor_admission_binds_public_route_not_execution_key() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let device_ura = crate::core::ura::device_ura("admission-test", "device-a");
        let agent_ura = crate::core::ura::agent_ura("admission-test", "local", "testbot");
        let authority =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
                device_ura,
                [agent_ura],
            )
            .expect("hosted Agent authority context");
        let mut catalog = crate::daemon::ability::dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority,
        );
        crate::daemon::ability::builtins::agents::discover::register_for_agent(
            &mut catalog,
            "testbot".to_string(),
            crate::daemon::persistence::agent_registry::AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let descriptor = catalog
            .public_descriptor_for_mode(
                &crate::daemon::ability::dispatch::OwnerKind::Agent("testbot".to_string()),
                "discover",
                crate::daemon::ability::CallMode::Rpc,
            )
            .expect("agent public descriptor");
        let ability_ura = descriptor
            .canonical_ability_ura()
            .expect("agent descriptor has canonical ability URA");
        let descriptor_ref = axon_sdk::invocation::canonical_ability_descriptor_ref(&format!(
            "{}@{}#{}!{}",
            ability_ura,
            descriptor.version,
            hex::encode(descriptor.descriptor_hash_bytes()),
            descriptor.admission_action().as_str()
        ))
        .expect("canonical agent descriptor ref");
        let facade = AdmissionFacade::with_trust_anchor_cell(
            SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default())),
            None,
        )
        .with_ability_catalog(Arc::new(catalog));

        facade
            .bound_admission_descriptor(
                "discover",
                crate::daemon::ability::CallMode::Rpc,
                &descriptor_ref,
            )
            .expect("public discover route must bind the qualified execution descriptor");

        facade
            .bound_admission_descriptor(
                "testbot.discover",
                crate::daemon::ability::CallMode::Rpc,
                &descriptor_ref,
            )
            .expect("hosted Agent dispatch key must normalize to owner-local public descriptor");
    }

    #[test]
    fn hosted_agent_key_provider_reaches_agent_policy_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let device_ura = crate::core::ura::device_ura("admission-test", "device-a");
        let agent_ura = crate::core::ura::agent_ura("admission-test", "alice", "testbot");
        let pending = crate::daemon::persistence::hosted_agent_publications::begin_registration(
            &agent_ura,
            &device_ura,
            1,
        )
        .expect("seed hosted publication intent");
        assert!(
            require_local_hosted_agent_publication_ready(&agent_ura, Some(&device_ura)).is_err(),
            "RegistrationPending must not be executable"
        );
        let publication_assignment =
            crate::daemon::federation::hosted_agent_publication::HostedAgentGenerationAssignment {
                agent_ura: agent_ura.clone(),
                host_device_ura: device_ura.clone(),
                incarnation_id: pending.incarnation_id().clone(),
                generation: 1,
            };
        crate::daemon::persistence::hosted_agent_publications::bind_assignment(
            &publication_assignment,
            2,
        )
        .expect("bind hosted publication assignment");
        assert!(
            require_local_hosted_agent_publication_ready(&agent_ura, Some(&device_ura)).is_err(),
            "Assigned must not be executable"
        );
        crate::daemon::persistence::hosted_agent_publications::stage_projection(
            &publication_assignment,
            pending.desired_catalog_epoch,
            1,
            "sha256:admission-ready",
            3,
        )
        .expect("stage hosted publication proof");
        assert!(
            require_local_hosted_agent_publication_ready(&agent_ura, Some(&device_ura)).is_err(),
            "Publishing must not be executable"
        );
        crate::daemon::persistence::hosted_agent_publications::mark_published(
            &publication_assignment,
            pending.desired_catalog_epoch,
            1,
            "sha256:admission-ready",
            4,
        )
        .expect("publish hosted Agent for policy test");
        require_local_hosted_agent_publication_ready(&agent_ura, Some(&device_ura))
            .expect("Published hosted Agent is executable");
        let authority =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
                device_ura.clone(),
                [agent_ura.clone()],
            )
            .expect("hosted Agent authority context");
        let mut catalog = crate::daemon::ability::dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority,
        );
        crate::daemon::ability::builtins::agents::discover::register_for_agent(
            &mut catalog,
            "testbot".to_string(),
            crate::daemon::persistence::agent_registry::AgentRegistry::default,
            Arc::new(std::sync::OnceLock::new()),
        );
        let descriptor = catalog
            .public_descriptor_for_mode(
                &crate::daemon::ability::dispatch::OwnerKind::Agent("testbot".to_string()),
                "discover",
                crate::daemon::ability::CallMode::Rpc,
            )
            .expect("agent public descriptor");
        let ability_ura = descriptor
            .canonical_ability_ura()
            .expect("agent descriptor has canonical ability URA");
        let descriptor_ref = axon_sdk::invocation::canonical_ability_descriptor_ref(&format!(
            "{}@{}#{}!{}",
            ability_ura,
            descriptor.version,
            hex::encode(descriptor.descriptor_hash_bytes()),
            descriptor.admission_action().as_str()
        ))
        .expect("canonical agent descriptor ref");
        let trust_anchor = RealmTrustAnchor::from_parts_with_principal_owners(
            Vec::new(),
            vec![crate::daemon::trust::anchor::TrustedPrincipalOwner {
                principal_ura: agent_ura.clone(),
                owner_user_id: "alice".to_string(),
                owner_ura: crate::core::ura::user_ura("admission-test", "alice"),
                added_at_unix_ms: 1,
            }],
            Vec::new(),
        )
        .expect("hosted Agent owner binding");
        let stores = Arc::new(AccessControlStoreRegistry::ephemeral());
        let signing_key = SigningKey::from_bytes(&[0x5a; 32]);
        let facade = AdmissionFacade::with_trust_anchor_cell(
            SharedTrustAnchor::new(Arc::new(trust_anchor)),
            Some(crate::core::ura::hub_ura("admission-test")),
        )
        .with_invocation_verification_keys(Arc::new(StaticInvocationVerificationKeyProvider {
            caller_ura: agent_ura.clone(),
            verifying_key: signing_key.verifying_key(),
        }))
        .with_access_control_stores(Arc::clone(&stores))
        .with_ability_catalog(Arc::new(catalog));
        let args = b"{}".to_vec();
        let descriptor_bound = DescriptorBoundEnvelope::new(InvocationEnvelope {
            caller: AgentIdentity::new(&agent_ura, UraProfile::StrictV2),
            callee: AgentIdentity::new(&agent_ura, UraProfile::StrictV2),
            subject: SubjectIdentity::new(
                crate::core::ura::resource_dot_ura(
                    "admission-test",
                    "user.alice",
                    "agent/discover",
                ),
                UraProfile::StrictV2,
            ),
            ability: descriptor_ref,
            args_digest: sha256(&args),
            invocation_nonce: [0x24; 16],
            causal_context: CausalContext::None,
        })
        .expect("descriptor-bound hosted Agent envelope");
        let caller_signature = axon_sdk::invocation::CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: vec![0x7b; ed25519_dalek::SIGNATURE_LENGTH],
            key_id_hint: "ed25519:hosted-agent".to_string(),
        };
        let envelope = runtime_admission_envelope(
            descriptor_bound.envelope(),
            Some(caller_signature),
            "req-hosted-agent".to_string(),
        )
        .expect("runtime wire envelope");
        let input = RuntimeAdmissionInput {
            facade: facade.clone(),
            envelope: envelope.clone(),
            ability: "discover".to_string(),
            arguments: args.clone(),
            metadata: HashMap::new(),
            call_mode: AxonCallMode::Rpc,
            ingress: RuntimeAdmissionIngress::CallerSigned,
        };

        let denied = match facade.reserve_runtime_admission(&input, &descriptor_bound) {
            Ok(_) => panic!("hosted Agent without Agent grant must not inherit owner allow"),
            Err(error) => error,
        };
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);
        assert!(
            denied.message().contains("\"principal_kind\":\"agent\""),
            "{denied:?}"
        );
        assert!(
            denied
                .message()
                .contains("\"reason\":\"NON_INTERACTIVE_DENY\""),
            "{denied:?}"
        );

        let action: AccessAction = descriptor.admission_action().into();
        stores
            .with_store("easynet:///r/admission-test/user/alice", |store| {
                store.create_grant(
                    crate::daemon::invocation::admission::grant_matcher::PermissionGrant {
                        grant_id: "facade-agent-grant".to_string(),
                        owner_user_ura: "easynet:///r/admission-test/user/alice".to_string(),
                        principal_kind: crate::daemon::invocation::admission::decision::PrincipalKind::Agent,
                        principal_id: agent_ura.clone(),
                        token_id: None,
                        token_class: None,
                        session_id: None,
                        session_expires_at: None,
                        callee_ura: Some(agent_ura.clone()),
                        subject_ura_pattern: Some(crate::core::ura::resource_dot_ura(
                            "admission-test",
                            "user.alice",
                            "agent/discover",
                        )),
                        ability_ura_pattern: Some(ability_ura),
                        actions: vec![action],
                        constraints: None,
                        effect: crate::daemon::invocation::admission::grant_matcher::PermissionEffect::Allow,
                        lifetime: crate::daemon::invocation::admission::grant_matcher::PermissionGrantLifetime::Permanent,
                        state: crate::daemon::invocation::admission::grant_matcher::PermissionGrantState::Active,
                        expires_at: None,
                        review_required_after: None,
                        last_reviewed_at: None,
                        last_used_at: None,
                        created_by: crate::core::ura::user_ura("admission-test", "alice"),
                        created_at: "2026-08-07T00:00:00Z".to_string(),
                        updated_at: None,
                        revoked_at: None,
                        reason: Some("facade hosted Agent regression".to_string()),
                    },
                    &crate::core::ura::user_ura("admission-test", "alice"),
                )
            })
            .expect("open policy store")
            .expect("create Agent grant");
        facade
            .reserve_runtime_admission(&input, &descriptor_bound)
            .expect("explicit Agent grant admits hosted Agent");
        crate::daemon::persistence::hosted_agent_publications::retire(
            &agent_ura,
            &publication_assignment.incarnation_id,
            publication_assignment.generation,
            5,
        )
        .expect("retire hosted Agent publication");
        assert!(
            require_local_hosted_agent_publication_ready(&agent_ura, Some(&device_ura)).is_err(),
            "Retired must not be executable"
        );

        for internal_owner in [
            "easynet:///r/test/authority",
            "easynet:///r/test/device/dev-1",
            "easynet:///r/test/agent/device.dev-1.runtime-introspection",
        ] {
            require_local_hosted_agent_publication_ready(internal_owner, None)
                .expect("non-user-Agent runtime owner bypasses hosted publication readiness");
        }

        let mapped = runtime_admission_status_to_axon(Status::permission_denied(
            "HOSTED_AGENT_NOT_PUBLISHED: assigned",
        ));
        assert_eq!(mapped.code, ErrorCode::AbilityDisabled);
        assert_eq!(mapped.stage, Some(ErrorStage::AbilityPolicy));
    }
}
