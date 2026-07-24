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
use serde::Deserialize;
use sha2::Digest;
use tonic::{Code, Status};

use axon_sdk::invocation::axiom::{authority_proof_expected_hash, KeyResolver};
use axon_sdk::invocation::{
    AuthorityBinding, AxonError as InvocationError, AxonErrorKind as InvocationErrorKind,
    CallMode as AxonCallMode, DelegationProofBody, DescriptorBoundEnvelope,
    InvocationAuthorityProof, SessionAuthorityBody, VerifiedAdmissionPolicy,
    REASON_CALLER_SIGNATURE_INVALID, REASON_ENVELOPE_INCOMPLETE, REASON_NONCE_REPLAY,
};

use crate::core::ura::{parse_ura, AbilitySelector, URAKind};
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
    AccessAction, PermissionRequestStatus, SignatureDecisionReason,
};
use crate::daemon::invocation::admission::federated_key_resolver::FederatedKeyResolver;
use crate::daemon::invocation::admission::grant_matcher::{
    GrantMatchInput, PermissionEffect, PermissionGrant, PermissionGrantMatcher,
};
use crate::daemon::invocation::admission::hosted_agent_publication::HostedAgentPublication;
use crate::daemon::invocation::admission::list_user_pubkeys::ABILITY_IDENTITY_LIST_USER_PUBKEYS;
use crate::daemon::invocation::admission::policy_gate::{
    ability_ura_for, principal_for, AdmissionPolicyContext, AdmissionPolicyGate,
};
use crate::daemon::invocation::admission::principal_lifecycle::{
    PrincipalAdmissionState, PrincipalLifecycleReader,
};
use crate::daemon::invocation::admission::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY;
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
use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgentRole};
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
const REASON_HOSTED_AGENT_DELEGATION_LOCAL_ONLY: &str = "HOSTED_AGENT_DELEGATION_LOCAL_ONLY";
const REASON_CALLER_UNKNOWN: &str = "CALLER_UNKNOWN";

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
    action: AccessAction,
    safe_read: bool,
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
    binding: AuthorityBinding,
    proof_type: &'static str,
    proof_payload: Vec<u8>,
}

impl VerifiedRuntimeAuthority {
    fn self_authority(principal_ura: impl Into<String>) -> Self {
        Self {
            authority_id: None,
            binding: AuthorityBinding::Self_ {
                principal_ura: principal_ura.into(),
            },
            proof_type: "self-authority",
            proof_payload: Vec::new(),
        }
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
            binding: AuthorityBinding::Bootstrap {
                principal_ura: envelope.caller.ura.clone(),
                realm,
                ability: envelope.ability.clone(),
            },
            proof_type: "bootstrap-authority",
            proof_payload: Vec::new(),
        })
    }

    fn delegated(verified: VerifiedSignedAuthority<DelegationPayload>) -> Result<Self, Status> {
        let authority_id = verified_delegation_authority_id(&verified.payload)?;
        Ok(Self {
            authority_id: Some(authority_id),
            binding: AuthorityBinding::Delegated(DelegationProofBody {
                issuer_ura: verified.payload.issuer_ura,
                subject_ura: verified.payload.subject_ura,
                caller_ura: verified.payload.caller_ura,
                audience: verified.payload.audience,
                scopes: verified.payload.scopes,
                issued_at_ms: verified.payload.issued_at_ms,
                expires_at_ms: verified.payload.expires_at_ms,
                signature: verified.signature,
            }),
            proof_type: "delegated-authority",
            proof_payload: verified.canonical_payload,
        })
    }

    fn session(verified: VerifiedSignedAuthority<SessionAuthorityPayload>) -> Result<Self, Status> {
        let parsed_subject = parse_ura(&verified.payload.subject_ura).map_err(|error| {
            Status::invalid_argument(format!(
                "session authority subject is not a canonical URA: {error}"
            ))
        })?;
        let user_ura = crate::core::ura::user_ura(
            &parsed_subject.realm,
            &verified.payload.session_owner_user_id,
        );
        Ok(Self {
            authority_id: Some(verified_session_authority_id(&verified.payload)),
            binding: AuthorityBinding::Session(SessionAuthorityBody {
                backend_ura: verified.payload.issuer_ura,
                user_ura,
                session_id: verified.payload.session_id,
                scopes: verified.payload.scopes,
                audiences: vec![verified.payload.audience],
                issued_at_ms: verified.payload.issued_at_ms,
                expires_at_ms: verified.payload.expires_at_ms,
                signature: verified.signature,
            }),
            proof_type: "session-authority",
            proof_payload: verified.canonical_payload,
        })
    }

    fn from_authority_proof(envelope: &Envelope, proof: &AuthorityProof) -> Result<Self, Status> {
        let caller_ura = caller_ura_required(envelope)?.to_string();
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
            binding: AuthorityBinding::Delegated(DelegationProofBody {
                issuer_ura: proof.issuer_ura.clone(),
                subject_ura: proof.subject_ura.clone(),
                caller_ura,
                audience: proof.audience_ura.clone(),
                scopes: vec![proof.ability_ura.clone()],
                issued_at_ms,
                expires_at_ms,
                signature,
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

const MAX_PENDING_RUNTIME_ADMISSIONS: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeAdmissionIngress {
    CallerSigned,
    ProvisionalBootstrap,
    TrustedLocalSystem,
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
        wire: &crate::daemon::axon_bridge::dispatch_shim::WireDispatch,
        ability: &str,
        call_mode: AxonCallMode,
    ) -> Result<DaemonRuntimeAdmissionLease, Status> {
        let caller_signature = match &wire.ingress {
            crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::ExternalSigned(_) => {
                Some(wire_caller_signature(wire)?)
            }
            crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::ProvisionalBootstrap(
                _,
            ) => Some(wire_caller_signature(wire)?),
            crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::LocalSystem => {
                None
            }
        };
        let ingress = match &wire.ingress {
            crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::ExternalSigned(_) => {
                RuntimeAdmissionIngress::CallerSigned
            }
            crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::ProvisionalBootstrap(
                _,
            ) => RuntimeAdmissionIngress::ProvisionalBootstrap,
            crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::LocalSystem => {
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
            axon_sdk::invocation::sha256(&descriptor_bound_canonical_bytes(&descriptor_bound));
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

    fn stage_derived(
        self: &Arc<Self>,
        facade: &AdmissionFacade,
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
        if descriptor_bound_canonical_bytes(&descriptor_bound)
            != descriptor_bound_canonical_bytes(&signed_descriptor_bound)
        {
            return Err(Status::invalid_argument(
                "derived runtime admission signed envelope does not match descriptor-bound request",
            ));
        }
        self.stage_canonical(
            facade,
            descriptor_bound,
            Some(signed_envelope.signature.clone()),
            arguments,
            metadata,
            request_id,
            ability,
            call_mode,
            RuntimeAdmissionIngress::CallerSigned,
        )
    }

    pub(crate) fn verify_provider_policy(
        &self,
        envelope: &DescriptorBoundEnvelope,
    ) -> Result<VerifiedAdmissionPolicy, InvocationError> {
        let envelope_key =
            axon_sdk::invocation::sha256(&descriptor_bound_canonical_bytes(&envelope));
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
    trusted_role: TrustedAgentRole,
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
    /// `None`.
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
            transport_boundary: AdmissionTransportBoundary::LocalOnlyIpc,
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
        let public_ability = public_ability_name_from_route(ability);
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
        Ok(BoundAdmissionDescriptor {
            action,
            safe_read: action == AccessAction::Read,
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

    /// Set the transport boundary that governs local self admission. Boot
    /// leaves the UDS-fed service on `LocalOnlyIpc` and clones the TCP/TLS-fed
    /// service with `OffBoxStrict`, so an off-box caller that spoofs the daemon
    /// URA cannot skip the strict trust-anchor / signature / replay pipeline.
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
            != descriptor_bound_canonical_bytes(&admitted_envelope)
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
        let caller_ura = caller_ura_required(&input.envelope)?;

        match input.ingress {
            RuntimeAdmissionIngress::TrustedLocalSystem => {
                if caller_ura != crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
                    return Err(Status::permission_denied(
                        "trusted local-system admission requires exact `_system.local` caller",
                    ));
                }
                return runtime_admission_decision(
                    admitted_envelope,
                    VerifiedRuntimeAuthority::self_authority(caller_ura),
                    RuntimeAdmissionReservation { quota: None },
                );
            }
            RuntimeAdmissionIngress::ProvisionalBootstrap => {
                Self::verify_provisional_federation_join(
                    &input.envelope,
                    &input.ability,
                    &input.arguments,
                )?;
                reject_public_hosted_agent_delegation_metadata(Some(&input.metadata))?;
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
                    VerifiedRuntimeAuthority::bootstrap(admitted_envelope.envelope(), None)?,
                    RuntimeAdmissionReservation { quota: None },
                );
            }
            RuntimeAdmissionIngress::CallerSigned => {}
        }

        let trust_anchor = self.trust_anchor.snapshot();
        let trusted_role = self
            .trusted_role_for_caller(caller_ura, trust_anchor.as_ref())
            .map_err(|status| {
                self.signature_denied_status(&input.envelope, &input.ability, status)
            })?;
        reject_public_hosted_agent_delegation_metadata(Some(&input.metadata))?;
        let authority = self.enforce_runtime_admitted_policy(
            &input.envelope,
            &input.ability,
            &input.arguments,
            Some(&input.metadata),
            trust_anchor,
            trusted_role,
            descriptor.action,
            descriptor.safe_read,
            &descriptor_bound,
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

    fn trusted_role_for_caller(
        &self,
        caller_ura: &str,
        trust_anchor: &RealmTrustAnchor,
    ) -> Result<TrustedAgentRole, Status> {
        if let Some(entry) = trust_anchor.lookup(caller_ura) {
            return Ok(entry.role);
        }
        if let Some(role) = self.principal_lifecycle_role_for_caller(caller_ura)? {
            return Ok(role);
        }
        if self.is_federated_caller(caller_ura) {
            return federated_caller_role(caller_ura)
                .ok_or_else(|| permission_denied_unknown_caller(caller_ura));
        }
        Err(permission_denied_unknown_caller(caller_ura))
    }

    fn principal_lifecycle_role_for_caller(
        &self,
        caller_ura: &str,
    ) -> Result<Option<TrustedAgentRole>, Status> {
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
            PrincipalAdmissionState::Active => Ok(Some(TrustedAgentRole::User)),
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
        trusted_role: TrustedAgentRole,
        action: AccessAction,
        safe_read: bool,
        descriptor_bound: &WireDescriptorBoundEnvelope,
    ) -> Result<VerifiedRuntimeAuthority, Status> {
        self.enforce_principal_lifecycle_admission(envelope, ability, trusted_role)
            .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
        if bootstrap_authority_ability(ability) {
            return VerifiedRuntimeAuthority::bootstrap(descriptor_bound.envelope.envelope(), None);
        }
        let metadata_authority = verify_delegation_metadata(
            envelope,
            ability,
            action,
            metadata,
            trust_anchor.as_ref(),
            current_unix_ms(),
        )
        .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
        let carries_authority_proof = metadata
            .and_then(|values| values.get(AUTHORITY_PROOF_METADATA_KEY))
            .is_some_and(|value| !value.trim().is_empty());
        if metadata_authority.is_some() && carries_authority_proof {
            return Err(self.authority_denied_status(
                envelope,
                ability,
                Status::invalid_argument(format!(
                    "{REASON_AUTHORITY_FORMAT_INVALID}: invocation carries multiple independent authority proofs"
                )),
            ));
        }
        let authority_proof_authority = self
            .verify_authority_proof_metadata(AuthorityProofMetadataInput {
                envelope,
                ability,
                action,
                metadata,
                trust_anchor: trust_anchor.as_ref(),
                trusted_role,
                descriptor_bound,
            })
            .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
        let bootstrap_authority_id = match BootstrapAuthorityVerifier::verify(
            envelope,
            ability,
            action,
            args,
            trust_anchor.as_ref(),
            trusted_role,
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
        let hosted_agent_publication_authority_id = self
            .verify_hosted_agent_publication_authority(
                envelope,
                ability,
                args,
                trust_anchor.as_ref(),
            )
            .map_err(|status| self.authority_denied_status(envelope, ability, status))?;
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
            .map(ToOwned::to_owned);
        AdmissionPolicyGate::verify(AdmissionPolicyContext {
            envelope,
            ability,
            action,
            safe_read,
            trusted_role,
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
            rejector_ura: self.daemon_ura.clone(),
        })?;
        if let Some(authority) = authority_proof_authority {
            return Ok(authority);
        }
        if let Some(authority) = metadata_authority {
            return Ok(authority);
        }
        if let Some(authority_id) = bootstrap_authority_id.or(hosted_agent_publication_authority_id)
        {
            return VerifiedRuntimeAuthority::bootstrap(
                descriptor_bound.envelope.envelope(),
                Some(authority_id),
            );
        }
        Ok(VerifiedRuntimeAuthority::self_authority(
            descriptor_bound.envelope.envelope().caller.ura.clone(),
        ))
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
        trusted_role: TrustedAgentRole,
    ) -> Result<(), Status> {
        if trusted_role != TrustedAgentRole::User {
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

    fn verify_provisional_federation_join(
        envelope: &Envelope,
        ability: &str,
        args: &[u8],
    ) -> Result<(), Status> {
        if ability != ABILITY_FEDERATION_JOIN {
            return Err(permission_denied_unknown_caller(
                envelope
                    .caller
                    .as_ref()
                    .map(|caller| caller.ura.as_str())
                    .unwrap_or("provisional:<missing>"),
            ));
        }
        let caller = caller_ura_required(envelope)?;
        let digest = caller.strip_prefix("provisional:").ok_or_else(|| {
            Status::invalid_argument("federation.join provisional caller missing prefix")
        })?;
        if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(Status::invalid_argument(
                "federation.join provisional caller must be `provisional:` plus 64 hex characters",
            ));
        }
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
        if request.realm != callee.realm || request.realm != subject.realm {
            return Err(Status::invalid_argument(format!(
                "federation.join realm mismatch: request={}, callee={}, subject={}",
                request.realm, callee.realm, subject.realm
            )));
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
        let presented_digest: [u8; 32] = hex::decode(digest)
            .map_err(|err| {
                Status::invalid_argument(format!(
                    "federation.join provisional caller digest is not hex: {err}"
                ))
            })?
            .try_into()
            .map_err(|bytes: Vec<u8>| {
                Status::invalid_argument(format!(
                    "federation.join provisional caller digest must decode to 32 bytes, got {}",
                    bytes.len()
                ))
            })?;
        let expected_digest: [u8; 32] = sha2::Sha256::digest(&public_key).into();
        if presented_digest != expected_digest {
            return Err(Status::permission_denied(
                "federation.join provisional caller does not match public_key_hex",
            ));
        }
        Ok(())
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
            trusted_role,
            descriptor_bound,
        } = input;
        let Some(proof) = authority_proof_from_metadata(metadata)? else {
            return Ok(None);
        };
        let caller_ura = caller_ura_required(envelope)?;
        let callee_ura = callee_ura_required(envelope)?;
        let subject_ura = subject_ura_required(envelope)?;
        let ability_ura = ability_ura_for(callee_ura, ability)?;
        let principal = principal_for(trusted_role, caller_ura, trust_anchor)?;
        let canonical_hash = format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(descriptor_bound_canonical_bytes(
                &descriptor_bound.envelope
            )))
        );
        let invocation_nonce = invocation_nonce_for_proof(envelope);
        let audience_ura = self.daemon_ura.as_deref().unwrap_or(callee_ura);
        let now = Utc::now();
        self.access_control_stores
            .with_store(&proof.owner_user_id, |store| {
                let resolver = StoreBackedAuthorityProofResolver {
                    trust_anchor,
                    store,
                    now,
                };
                let context = AuthorityProofVerificationContext {
                    owner_user_id: &proof.owner_user_id,
                    principal_kind: principal.kind,
                    principal_id: &principal.id,
                    token_id: principal.token_id.as_deref(),
                    callee_ura,
                    subject_ura,
                    ability_ura: &ability_ura,
                    action,
                    nonce: invocation_nonce.as_deref(),
                    canonical_hash: Some(canonical_hash.as_str()),
                    audience_ura,
                    session_id: proof.session_id.as_deref(),
                    session_owner_user_id: proof.session_owner_user_id.as_deref(),
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
                    proof.owner_user_id
                ))
            })??;
        VerifiedRuntimeAuthority::from_authority_proof(envelope, &proof).map(Some)
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

    pub(crate) fn stage(
        &self,
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
    wire: &crate::daemon::axon_bridge::dispatch_shim::WireDispatch,
) -> Result<axon_sdk::invocation::CallerSignature, Status> {
    match &wire.ingress {
        crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::ExternalSigned(
            signature,
        )
        | crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::ProvisionalBootstrap(
            signature,
        ) => Ok(signature.clone()),
        crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::LocalSystem => Err(
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
        Code::Cancelled => InvocationError::cancelled(detail),
        Code::DeadlineExceeded => InvocationError::deadline_exceeded(detail),
        Code::InvalidArgument | Code::OutOfRange => InvocationError::invalid_argument(detail),
        Code::ResourceExhausted => InvocationError::resource_exhausted(detail),
        Code::Unavailable => InvocationError::unavailable(detail),
        Code::PermissionDenied | Code::Unauthenticated => {
            InvocationError::permission_denied(detail)
        }
        Code::Ok
        | Code::Unknown
        | Code::NotFound
        | Code::AlreadyExists
        | Code::FailedPrecondition
        | Code::Aborted
        | Code::Unimplemented
        | Code::Internal
        | Code::DataLoss => InvocationError::internal(detail),
    }
}

struct StoreBackedAuthorityProofResolver<'a> {
    trust_anchor: &'a RealmTrustAnchor,
    store: &'a AccessControlStore,
    now: DateTime<Utc>,
}

impl AuthorityProofIssuerResolver for StoreBackedAuthorityProofResolver<'_> {
    fn verifying_key_for_issuer(&self, issuer_ura: &str) -> Option<VerifyingKey> {
        let entry = self.trust_anchor.lookup(issuer_ura)?;
        let bytes = BASE64_STANDARD
            .decode(entry.public_key_b64.as_bytes())
            .ok()?;
        let bytes: [u8; ed25519_dalek::PUBLIC_KEY_LENGTH] = bytes.try_into().ok()?;
        VerifyingKey::from_bytes(&bytes).ok()
    }

    fn issuer_authorized_for_owner(&self, issuer_ura: &str, owner_user_id: &str) -> bool {
        let Some(entry) = self.trust_anchor.lookup(issuer_ura) else {
            return false;
        };
        if entry.role != TrustedAgentRole::User {
            return false;
        }
        crate::core::ura::parse_ura(issuer_ura)
            .ok()
            .and_then(|parsed| parsed.user_id().map(ToOwned::to_owned))
            .is_some_and(|user_id| user_id == owner_user_id)
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
        owner_user_id: &proof.owner_user_id,
        principal_kind: proof.principal_kind,
        principal_id: &proof.principal_id,
        token_id: proof.token_id.as_deref(),
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

/// **PR-N2 commit 1/N**. Parse the realm component from a canonical
/// EasyNet URA (`easynet:///r/<realm>/...`). Returns the realm slice
/// when the shape matches, `None` otherwise. Shared by
/// `is_federated_caller` and the cross-realm gate.
///
/// Important: federated callers are not uniformly `.../agent/...`;
/// peer hubs use Axon's canonical hub identity shape and device sessions
/// register under `.../device/<id>`. Realm projection goes through the
/// core URA facade so all canonical role tails stay accepted and retired
/// aliases stay rejected.

fn federated_caller_role(caller_ura: &str) -> Option<TrustedAgentRole> {
    let parsed = parse_ura(caller_ura).ok()?;
    match parsed.kind {
        URAKind::Device => Some(TrustedAgentRole::Device),
        URAKind::User => Some(TrustedAgentRole::User),
        URAKind::Authority => Some(TrustedAgentRole::Hub),
        _ => None,
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Bootstrap authority abilities mutate identity or presence roots.
/// They still require the caller to pass strict admission above; this
/// gate only keeps trust-anchor bootstrap out of normal user-delegation
/// semantics so stale backend issuer keys cannot deadlock key repair.
fn bootstrap_authority_ability(ability: &str) -> bool {
    matches!(
        ability,
        ABILITY_IDENTITY_REGISTER_PUBKEY
            | ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
            | ABILITY_IDENTITY_LIST_USER_PUBKEYS
            | ABILITY_IDENTITY_REVOKE_USER_PUBKEY
    )
}

fn public_ability_name_from_route(ability: &str) -> String {
    let trimmed = ability.trim();
    let ability_ura = trimmed
        .split_once('@')
        .map(|(ability_ura, _)| ability_ura)
        .unwrap_or(trimmed);
    AbilitySelector::parse(ability_ura)
        .map(|selector| selector.public_name().to_string())
        .unwrap_or_else(|_| trimmed.to_string())
}

#[derive(Debug, Deserialize)]
struct DelegationProofRaw {
    payload: serde_json::Value,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct SessionAuthorityRaw {
    payload: serde_json::Value,
    signature: String,
}

fn verify_delegation_metadata(
    envelope: &Envelope,
    ability: &str,
    action: AccessAction,
    metadata: Option<&HashMap<String, String>>,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
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
            let verified =
                parse_and_verify_delegation_proof(raw_proof, trust_anchor, now_ms)?;
            verify_delegation_bindings(&verified.payload, envelope, ability)?;
            VerifiedRuntimeAuthority::delegated(verified).map(Some)
        }
        (None, Some(raw_session)) => {
            let verified =
                parse_and_verify_session_authority(raw_session, trust_anchor, now_ms)?;
            verify_session_authority_bindings(&verified.payload, envelope, ability, action)?;
            VerifiedRuntimeAuthority::session(verified).map(Some)
        }
        (None, None) => {
            if envelope_requires_authority(envelope) {
                return Err(Status::permission_denied(format!(
                    "{REASON_AUTHORITY_REQUIRED}: envelope subject differs from caller and is a user/session authority subject; \
                     missing `{DELEGATION_METADATA_KEY}` or `{SESSION_AUTHORITY_METADATA_KEY}` metadata"
                )));
            }
            Ok(None)
        }
    }
}

fn parse_and_verify_session_authority(
    raw_authority: &str,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
) -> Result<VerifiedSignedAuthority<SessionAuthorityPayload>, Status> {
    let wire = BASE64_STANDARD.decode(raw_authority).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority base64 decode failed: {err}"
        ))
    })?;

    let raw: SessionAuthorityRaw = serde_json::from_slice(&wire).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: raw session authority JSON parse failed: {err}"
        ))
    })?;

    let payload: SessionAuthorityPayload = serde_json::from_value(raw.payload).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority payload parse failed: {err}"
        ))
    })?;
    authority_metadata::validate_session_authority_payload_shape(&payload, Some(now_ms))
        .map_err(authority_metadata_error_status)?;

    let payload_bytes = authority_metadata::canonical_authority_payload_bytes(&payload)
        .map_err(authority_metadata_error_status)?;
    let signature = BASE64_STANDARD.decode(&raw.signature).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: session authority signature base64 decode failed: {err}"
        ))
    })?;

    let issuer = trust_anchor.lookup(&payload.issuer_ura).ok_or_else(|| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_UNKNOWN}: session authority issuer `{}` is not in the realm \
             trust anchor",
            payload.issuer_ura
        ))
    })?;
    verify_delegation_signature(&issuer.public_key_b64, &payload_bytes, &signature)?;

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
            "{REASON_AUTHORITY_SUBJECT_MISMATCH}: session subject `{}` owned by `{}` does not \
             admit envelope subject `{subject}`",
            payload.subject_ura, payload.session_owner_user_id
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

fn parse_and_verify_delegation_proof(
    raw_proof: &str,
    trust_anchor: &RealmTrustAnchor,
    now_ms: i64,
) -> Result<VerifiedSignedAuthority<DelegationPayload>, Status> {
    let wire = BASE64_STANDARD.decode(raw_proof).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: metadata base64 decode failed: {err}"
        ))
    })?;

    let raw: DelegationProofRaw = serde_json::from_slice(&wire).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: raw proof JSON parse failed: {err}"
        ))
    })?;

    let payload: DelegationPayload = serde_json::from_value(raw.payload).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority payload parse failed: {err}"
        ))
    })?;
    authority_metadata::validate_delegation_payload_shape(&payload, Some(now_ms))
        .map_err(authority_metadata_error_status)?;

    let payload_bytes = authority_metadata::canonical_authority_payload_bytes(&payload)
        .map_err(authority_metadata_error_status)?;
    let signature = BASE64_STANDARD.decode(&raw.signature).map_err(|err| {
        Status::invalid_argument(format!(
            "{REASON_AUTHORITY_FORMAT_INVALID}: authority signature base64 decode failed: {err}"
        ))
    })?;

    let issuer = trust_anchor.lookup(&payload.issuer_ura).ok_or_else(|| {
        Status::permission_denied(format!(
            "{REASON_AUTHORITY_ISSUER_UNKNOWN}: authority issuer `{}` is not in the realm \
             trust anchor",
            payload.issuer_ura
        ))
    })?;
    verify_delegation_signature(&issuer.public_key_b64, &payload_bytes, &signature)?;

    Ok(VerifiedSignedAuthority {
        payload,
        canonical_payload: payload_bytes,
        signature,
    })
}

fn envelope_requires_authority(envelope: &Envelope) -> bool {
    let Some(caller) = envelope.caller.as_ref().map(|c| c.ura.as_str()) else {
        return false;
    };
    let Some(subject) = envelope.subject.as_ref().map(|s| s.ura.as_str()) else {
        return false;
    };
    if caller == subject {
        return false;
    }
    matches!(
        authority_metadata::authority_subject_kind(subject),
        AuthoritySubjectKind::User | AuthoritySubjectKind::Session
    )
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

fn verify_delegation_bindings(
    payload: &DelegationPayload,
    envelope: &Envelope,
    ability: &str,
) -> Result<(), Status> {
    let caller = caller_ura_required(envelope)?;
    if payload.caller_ura != caller {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_CALLER_MISMATCH}: authority caller `{}` does not match envelope \
             caller `{caller}`",
            payload.caller_ura
        )));
    }

    let subject = subject_ura_required(envelope)?;
    if payload.subject_ura != subject {
        return Err(Status::permission_denied(format!(
            "{REASON_AUTHORITY_SUBJECT_MISMATCH}: authority subject `{}` does not match envelope \
             subject `{subject}`",
            payload.subject_ura
        )));
    }

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
    envelope
        .caller
        .as_ref()
        .map(|c| c.ura.trim())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{REASON_ENVELOPE_INCOMPLETE}: envelope.caller.ura is required \
                 (Invariant 1: caller URA required)"
            ))
        })
}

/// Extract `callee.ura` and reject as `invalid_argument` if absent
/// or empty. Authority verification must not synthesize an audience
/// from an incomplete canonical tuple.
fn callee_ura_required(envelope: &Envelope) -> Result<&str, Status> {
    envelope
        .callee
        .as_ref()
        .map(|c| c.ura.trim())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{REASON_ENVELOPE_INCOMPLETE}: envelope.callee.ura is required \
                 (Invariant 1: callee URA required)"
            ))
        })
}

/// Extract `subject.ura` and reject as `invalid_argument` if absent
/// or empty. Authority verification must compare explicit subject
/// facts rather than treating a missing subject as an empty owner.
fn subject_ura_required(envelope: &Envelope) -> Result<&str, Status> {
    envelope
        .subject
        .as_ref()
        .map(|s| s.ura.trim())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(format!(
                "{REASON_ENVELOPE_INCOMPLETE}: envelope.subject.ura is required \
                 (Invariant 1: subject URA required)"
            ))
        })
}

fn current_unix_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn permission_denied_unknown_caller(caller_ura: &str) -> Status {
    Status::permission_denied(format!(
        "{REASON_CALLER_UNKNOWN}: caller URA `{caller_ura}` is not in the canonical local \
         PrincipalLifecycle aggregate or realm trust-anchor projection",
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
    use serde_json::json;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    const TEST_DESCRIPTOR_REF: &str = "easynet:///r/policy/ability/policy.worker.run@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke";

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
    }

    fn assert_complete_non_self_policy(
        authority: VerifiedRuntimeAuthority,
        envelope: &DescriptorBoundEnvelope,
        expected_form: &str,
    ) {
        assert_eq!(authority.binding.form(), expected_form);
        assert_ne!(authority.binding.form(), "self");
        let proof = authority.authority_proof(envelope);
        assert_eq!(proof.binding.as_ref(), Some(&authority.binding));
        assert_ne!(proof.proof_hash, [0u8; 32]);
        assert_eq!(proof.proof_hash, authority_proof_expected_hash(&proof));
        proof
            .validate_complete()
            .expect("verified product authority must produce complete receipt proof facts");
        authority
            .into_policy(envelope)
            .expect("complete product authority must construct canonical admission policy");
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

        assert_complete_non_self_policy(authority, &envelope, "delegated");
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

        assert_complete_non_self_policy(authority, &envelope, "session");
    }

    #[test]
    fn session_authority_binding_requires_explicit_envelope_subject() {
        let caller_ura = "easynet:///r/policy/authority";
        let callee_ura = "easynet:///r/policy/agent/service.worker";
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
        let resolver = Arc::new(FederatedKeyResolver::new(
            trust.clone(),
            Some(Arc::new(RejectingFederationClient)),
            SharedFederatedPeers::new(peers),
            Some("self-realm".to_string()),
        ));
        AdmissionFacade::with_trust_anchor_cell(
            trust,
            Some(crate::core::ura::hub_ura("self-realm")),
        )
        .with_federated_key_resolver(resolver)
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
                .trusted_role_for_caller(user, &RealmTrustAnchor::default())
                .expect("active lifecycle user should be trusted as User"),
            TrustedAgentRole::User
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
    fn off_box_facade_does_not_accept_local_system_self_admission() {
        assert!(
            !AdmissionTransportBoundary::OffBoxStrict.accepts_local_self_caller(
                Some("easynet:///r/test/device/daemon"),
                crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA,
            )
        );
    }

    #[test]
    fn federated_caller_classification_accepts_only_canonical_peer_identities() {
        let facade = federated_facade();

        assert!(facade.is_federated_caller("easynet:///r/peer-realm/authority"));
        assert!(!facade.is_federated_caller("easynet:///r/peer-realm/authority/extra"));
        assert!(!facade.is_federated_caller("easynet:///r/self-realm/authority"));
        assert!(!facade.is_federated_caller("easynet:///r/unknown-realm/authority"));
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
    }
}
