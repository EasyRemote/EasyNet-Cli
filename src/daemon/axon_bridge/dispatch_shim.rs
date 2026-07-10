//! LocalRuntime dispatch adapter: route daemon ingress through Axon's
//! public descriptor-bound APIs.
//!
//! Goal
//! ----
//! Collapse the existing `dispatch_invoke_remote` /
//! `dispatch_self_targeted_invoke_remote` /
//! `dispatch_self_targeted_forward_invoke` family into **one** path:
//!
//!   1. Take the wire pieces (`pb::axon::v1::Envelope` from
//!      `EnvelopeOpen.envelope` + `target.ability_name` from
//!      `EnvelopeOpen.target` + `initial_args` bytes from
//!      `EnvelopeOpen.initial_args` + the signed-caller envelope's
//!      `caller_signature`).
//!   2. Reassemble the canonical 7-tuple via Axon's
//!      `InvocationEnvelope::from_wire_parts` — the helper Axon ships
//!      precisely so every wire-receiving consumer agrees on what
//!      "the envelope" means.
//!   3. Classify the ingress as either external signed caller material or a
//!      daemon-local system call, then hand the request to Axon's public
//!      `DescriptorBoundInvocationRequest::{externally_signed,signed}`
//!      constructors via `LocalRuntimeRequestFactory`.
//!
//! After Phase 4: there is no longer a "self-target shortcut" that
//! looks different from a "remote-target" path inside the daemon.
//! Both go through `LocalRuntime`; the only thing that varies is
//! whether the ability's handler runs in-process (because it was
//! registered) or the runtime returns
//! `AxonError::invalid_argument("unknown_ability:...")` (the
//! universal "I don't host this" signal).

use std::collections::HashMap;
use std::sync::Arc;

use easynet_axon::invocation::{
    fresh_nonce, AgentIdentity, AxonError, BidiInvocationHandle,
    CallMode as AxonInvocationCallMode, CallerSignature, CausalContext, DescriptorBoundEnvelope,
    DescriptorBoundEnvelopeParts, DescriptorBoundInvocationRequest, EntityRef, InvocationHandle,
    InvocationState, LocalRuntime, StreamingInvocationHandle, SubjectIdentity, UraProfile,
};

use easynet_axon::pb::axon::v1 as pb;

use crate::daemon::axon_bridge::descriptor_ref::{
    ability_descriptor_ref_for_wire, ability_ura_for_wire, registered_descriptor_version,
};
use crate::daemon::axon_bridge::local_runtime_request::{
    LocalRuntimeIngress, LocalRuntimeRequestFactory, LocalRuntimeRequestOptions,
};
use crate::daemon::axon_bridge::wire_descriptor::{
    descriptor_bound_from_wire_parts, WireCallerIdentity,
};
use crate::daemon::identity::local_invocation::system_agent_identity;

/// Explicit admission class for a wire-shaped LocalRuntime dispatch.
#[derive(Debug)]
pub enum WireDispatchIngress {
    /// Caller-supplied signature that Axon must verify.
    ExternalSigned(CallerSignature),
    /// Explicit daemon/session-local dispatch signed by `_system.local`.
    LocalSystem,
}

/// Canonical wire dispatch for descriptor-bound LocalRuntime calls.
#[derive(Debug)]
pub struct WireDispatch {
    pub envelope: DescriptorBoundEnvelope,
    pub ingress: WireDispatchIngress,
    pub payload: Vec<u8>,
    /// Transport metadata admitted by the daemon/session boundary and
    /// preserved for envelope-aware product handlers.
    pub request_metadata: HashMap<String, String>,
    /// Operational trace-correlation id from the wire envelope
    /// (empty when the caller sent none). Threaded into the Axon
    /// runtime so the ledger record carries it.
    pub trace_id: String,
}

#[derive(Clone, Copy)]
enum WireIngressPolicy {
    ExternalSigned,
    LocalSystem,
}

fn dispatch_from_wire_parts(
    envelope: pb::Envelope,
    target_ability_name: String,
    initial_args: Vec<u8>,
    request_metadata: HashMap<String, String>,
    policy: WireIngressPolicy,
) -> Result<WireDispatch, Box<AxonError>> {
    let caller_identity = match policy {
        WireIngressPolicy::ExternalSigned => WireCallerIdentity::FromEnvelope,
        WireIngressPolicy::LocalSystem => WireCallerIdentity::LocalSystem,
    };
    let reassembled = descriptor_bound_from_wire_parts(
        envelope.clone(),
        target_ability_name,
        &initial_args,
        caller_identity,
    )
    .map_err(Box::new)?;
    let ingress = match policy {
        WireIngressPolicy::ExternalSigned => {
            let signature = envelope
                .caller_signature
                .clone()
                .ok_or_else(|| {
                    AxonError::invalid_argument("wire envelope missing caller_signature")
                })?
                .into();
            WireDispatchIngress::ExternalSigned(signature)
        }
        WireIngressPolicy::LocalSystem => WireDispatchIngress::LocalSystem,
    };

    Ok(WireDispatch {
        envelope: reassembled.envelope,
        ingress,
        payload: initial_args,
        request_metadata,
        trace_id: reassembled.trace_id,
    })
}

/// Reassemble an externally signed wire envelope into the canonical
/// descriptor-bound dispatch object.
///
/// The caller signature is mandatory. Daemon product policy may reject or
/// authorize a route before this point, but Axon admission still owns
/// signature structure, crypto verification, nonce replay, and receipt proof
/// normalization.
// Err is boxed (clippy result_large_err): AxonError is ≥144 B, and the
// Ok path of every dispatch would otherwise carry the large variant.
pub fn external_signed_from_wire_parts(
    envelope: pb::Envelope,
    target_ability_name: String,
    initial_args: Vec<u8>,
    request_metadata: HashMap<String, String>,
) -> Result<WireDispatch, Box<AxonError>> {
    dispatch_from_wire_parts(
        envelope,
        target_ability_name,
        initial_args,
        request_metadata,
        WireIngressPolicy::ExternalSigned,
    )
}

/// Reassemble an explicit daemon/session-local dispatch from the same wire
/// envelope pieces, then bind it to the synthetic `_system.local`
/// caller before entering Axon's public signed request path.
///
/// Use this only after an outer daemon/session policy gate has accepted the
/// request. Public carrier ingress that still depends on caller crypto must
/// use [`external_signed_from_wire_parts`] instead.
pub fn local_system_from_wire_parts(
    envelope: pb::Envelope,
    target_ability_name: String,
    initial_args: Vec<u8>,
    request_metadata: HashMap<String, String>,
) -> Result<WireDispatch, Box<AxonError>> {
    dispatch_from_wire_parts(
        envelope,
        target_ability_name,
        initial_args,
        request_metadata,
        WireIngressPolicy::LocalSystem,
    )
}

fn request_for_wire_dispatch(
    mode: AxonInvocationCallMode,
    wire: WireDispatch,
) -> Result<DescriptorBoundInvocationRequest, AxonError> {
    let WireDispatch {
        envelope,
        ingress,
        payload,
        request_metadata,
        trace_id,
    } = wire;
    let ingress = match ingress {
        WireDispatchIngress::ExternalSigned(signature) => LocalRuntimeIngress::ExternalSigned {
            envelope,
            signature,
            payload,
        },
        WireDispatchIngress::LocalSystem => LocalRuntimeIngress::LocalSystem { envelope, payload },
    };
    LocalRuntimeRequestFactory::request_for(
        mode,
        ingress,
        LocalRuntimeRequestOptions::default()
            .with_trace_id(trace_id)
            .with_request_metadata(request_metadata),
    )
}

pub fn external_signed_from_envelope_open(
    envelope_open: &pb::EnvelopeOpen,
) -> Result<WireDispatch, Box<AxonError>> {
    let ability = envelope_open
        .target
        .as_ref()
        .map(|target| target.ability_name.clone())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing target ability"))?;
    let envelope = envelope_open
        .envelope
        .clone()
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing envelope"))?;
    external_signed_from_wire_parts(
        envelope,
        ability,
        envelope_open.initial_args.clone(),
        envelope_open.metadata.clone(),
    )
}

/// One terminal outcome from an Axon-driven RPC dispatch.
///
/// `payload_bytes` is the bytes the ability handler returned on
/// `Completed`; empty on a non-completed terminal. `error` is
/// `None` iff the terminal state was `Completed`.
///
/// `AxonError` does not impl `Eq`, so this struct stops at `Debug`
/// + `Clone`. Equality checks in tests compare individual fields.
#[derive(Debug, Clone)]
pub struct RpcDispatchOutcome {
    pub invocation_id: Option<String>,
    pub state: InvocationState,
    pub payload_bytes: Vec<u8>,
    pub error: Option<AxonError>,
    /// Admission receipt minted by Axon for the descriptor-bound call,
    /// when admission reached the runtime.
    pub admission_receipt: Option<easynet_axon::invocation::InvocationReceipt>,
    /// Terminal execution receipt, when the runtime minted one —
    /// carried back to the hub on carrier-v1 sessions (DEC-F004).
    pub terminal_receipt: Option<easynet_axon::invocation::InvocationReceipt>,
}

/// Drain an `InvocationHandle` to its terminal state and project
/// the last event into an `RpcDispatchOutcome`. The runtime's
/// LedgerSink (wired at boot) persists the canonical record on the
/// same task; we just need to surface the result back to the
/// wire-layer caller.
async fn drain_to_outcome(handle: InvocationHandle) -> RpcDispatchOutcome {
    let state = handle.wait().await;
    let events = handle.core().snapshot_events().await;
    let terminal = events.iter().rev().find(|e| e.state.is_terminal()).cloned();
    let receipts = handle.core().snapshot_receipts().await;
    let admission_receipt = receipts
        .iter()
        .find(|r| r.state == InvocationState::Admitted)
        .cloned();
    let terminal_receipt = receipts.into_iter().rev().find(|r| r.state.is_terminal());

    match (state, terminal) {
        (InvocationState::Completed, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id),
            state,
            payload_bytes: ev.payload,
            error: None,
            admission_receipt: admission_receipt.clone(),
            terminal_receipt: terminal_receipt.clone(),
        },
        (InvocationState::Failed, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id.clone()),
            state,
            payload_bytes: Vec::new(),
            error: Some(
                AxonError::internal(ev.reason.clone()).with_invocation_id(ev.invocation_id),
            ),
            admission_receipt: admission_receipt.clone(),
            terminal_receipt: terminal_receipt.clone(),
        },
        (InvocationState::TimedOut, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id.clone()),
            state,
            payload_bytes: Vec::new(),
            error: Some(
                AxonError::deadline_exceeded(ev.reason.clone())
                    .with_invocation_id(ev.invocation_id),
            ),
            admission_receipt: admission_receipt.clone(),
            terminal_receipt: terminal_receipt.clone(),
        },
        (InvocationState::Cancelled, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id.clone()),
            state,
            payload_bytes: Vec::new(),
            error: Some(
                AxonError::cancelled(ev.reason.clone()).with_invocation_id(ev.invocation_id),
            ),
            admission_receipt: admission_receipt.clone(),
            terminal_receipt: terminal_receipt.clone(),
        },
        // No terminal event recorded — should not happen because
        // wait() returned, but treat defensively.
        (_, None) => RpcDispatchOutcome {
            invocation_id: None,
            state,
            payload_bytes: Vec::new(),
            error: Some(AxonError::internal(
                "axon dispatch ended without a terminal event",
            )),
            admission_receipt,
            terminal_receipt: None,
        },
        // Wait() returned a non-terminal state — defensive.
        (other, _) => RpcDispatchOutcome {
            invocation_id: None,
            state: other,
            payload_bytes: Vec::new(),
            error: Some(AxonError::internal(format!(
                "axon dispatch ended in non-terminal state {}",
                other.as_str()
            ))),
            admission_receipt,
            terminal_receipt: None,
        },
    }
}

/// Phase-4 unary entry. Translates the wire-shape inputs into an
/// Axon invocation, admits + dispatches, and returns the terminal
/// outcome.
///
/// Failure shape is **always** an `RpcDispatchOutcome` — `error`
/// is `None` on success, `Some` for every operational failure
/// (target unknown, signature invalid, handler returned Err,
/// timed out, cancelled). The wire-shape mapping (in-band frame
/// vs gRPC `Status`) is the *caller's* responsibility; this shim
/// only speaks in SDK types, matching the broader Phase-4 invariant
/// that admission / dispatch / audit / persist live in Axon and
/// CLI owns only the transport translation.
pub async fn dispatch_rpc(runtime: &Arc<LocalRuntime>, wire: WireDispatch) -> RpcDispatchOutcome {
    let request = match request_for_wire_dispatch(AxonInvocationCallMode::Rpc, wire) {
        Ok(request) => request,
        Err(err) => {
            return RpcDispatchOutcome {
                invocation_id: None,
                state: InvocationState::Failed,
                payload_bytes: Vec::new(),
                error: Some(err),
                admission_receipt: None,
                terminal_receipt: None,
            };
        }
    };
    let result = runtime.invoke_descriptor_bound_request_async(request).await;
    match result {
        Ok((handle, _signed)) => drain_to_outcome(handle).await,
        Err(err) => RpcDispatchOutcome {
            invocation_id: None,
            state: InvocationState::Failed,
            payload_bytes: Vec::new(),
            error: Some(err),
            admission_receipt: None,
            terminal_receipt: None,
        },
    }
}

/// Unary entry for externally signed callers after daemon policy routing.
pub async fn dispatch_rpc_external_signed(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> RpcDispatchOutcome {
    dispatch_rpc(runtime, wire).await
}

/// Unary entry for daemon-admitted externally signed callers.
pub async fn dispatch_rpc_admitted(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> RpcDispatchOutcome {
    dispatch_rpc(runtime, wire).await
}

/// Unary entry for explicit daemon/session-local system dispatch.
pub async fn dispatch_rpc_local_system(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> RpcDispatchOutcome {
    dispatch_rpc(runtime, wire).await
}

/// Dispatch a node-internal `runtime.*` admin ability by its BARE
/// registered name, bypassing descriptor-bound canonicalization.
///
/// Admin abilities (`runtime.bootstrap_self_identity`, …) are installed by
/// the Axon SDK under their bare name with no owner, no descriptor proof,
/// and no presence/control-plane record. Routing them through the
/// descriptor-bound path would canonicalize the name to a device-owned
/// ability URA (`device.<id>.runtime.…`) the runtime never registered and
/// demand a proof binding the admin handler does not carry — both wrong for
/// a runtime-internal handshake. `invoke_async` runs the bare name under a
/// system-local binding, which is exactly the admin contract.
pub async fn dispatch_rpc_local_admin_bare(
    runtime: &Arc<LocalRuntime>,
    ability: &str,
    payload: Vec<u8>,
) -> RpcDispatchOutcome {
    match runtime.invoke_async(ability, payload, None, None).await {
        Ok(handle) => drain_to_outcome(handle).await,
        Err(err) => RpcDispatchOutcome {
            state: InvocationState::Failed,
            payload_bytes: Vec::new(),
            error: Some(err),
            invocation_id: None,
            admission_receipt: None,
            terminal_receipt: None,
        },
    }
}

pub async fn open_stream_external_signed(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> Result<StreamingInvocationHandle, AxonError> {
    open_stream(runtime, wire).await
}

/// Server-stream entry for daemon-admitted externally signed callers.
pub async fn open_stream_admitted(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> Result<StreamingInvocationHandle, AxonError> {
    open_stream(runtime, wire).await
}

async fn open_stream(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> Result<StreamingInvocationHandle, AxonError> {
    let request = request_for_wire_dispatch(AxonInvocationCallMode::Stream, wire)?;
    let (handle, _signed) = runtime
        .invoke_descriptor_bound_stream_request_async(request)
        .await?;
    Ok(handle)
}

fn local_descriptor_subject(
    requested_subject_ura: &str,
) -> Result<(SubjectIdentity, EntityRef), AxonError> {
    let requested = SubjectIdentity::new(
        requested_subject_ura.to_string(),
        UraProfile::EasynetStrictV2,
    );
    let subject_ref = EntityRef::try_from_subject_identity(&requested).map_err(|err| {
        AxonError::invalid_argument(format!(
            "local dispatch subject `{requested_subject_ura}` is not descriptor-bound: {err}"
        ))
    })?;
    Ok((requested, subject_ref))
}

pub async fn open_stream_local_with_subject(
    runtime: &Arc<LocalRuntime>,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: Vec<u8>,
) -> Result<StreamingInvocationHandle, AxonError> {
    let caller = system_agent_identity();
    let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
    let (subject, _) = local_descriptor_subject(subject_ura)?;
    let runtime_ability = ability_ura_for_wire(callee_ura, ability)?;
    let descriptor_version =
        registered_descriptor_version(runtime, &runtime_ability, AxonInvocationCallMode::Stream)
            .await?;
    let ability = ability_descriptor_ref_for_wire(callee_ura, ability, &descriptor_version)?;
    let descriptor_bound = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller,
        callee,
        ability,
        subject,
        invocation_nonce: fresh_nonce(),
        causal_context: CausalContext::None,
        args_bytes: &args,
    })?;
    let request = LocalRuntimeRequestFactory::request_for(
        AxonInvocationCallMode::Stream,
        LocalRuntimeIngress::LocalSystem {
            envelope: descriptor_bound,
            payload: args,
        },
        LocalRuntimeRequestOptions::default(),
    )?;
    let (handle, _signed) = runtime
        .invoke_descriptor_bound_stream_request_async(request)
        .await?;
    Ok(handle)
}

pub async fn open_bidi_external_signed(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> Result<BidiInvocationHandle, AxonError> {
    open_bidi(runtime, wire).await
}

/// Bidi entry for daemon-admitted externally signed callers.
pub async fn open_bidi_admitted(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> Result<BidiInvocationHandle, AxonError> {
    open_bidi(runtime, wire).await
}

async fn open_bidi(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
) -> Result<BidiInvocationHandle, AxonError> {
    let request = request_for_wire_dispatch(AxonInvocationCallMode::Bidi, wire)?;
    let (handle, _signed) = runtime
        .invoke_descriptor_bound_bidi_request_async(request)
        .await?;
    Ok(handle)
}

pub async fn open_bidi_local_with_subject(
    runtime: &Arc<LocalRuntime>,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: Vec<u8>,
) -> Result<BidiInvocationHandle, AxonError> {
    let caller = system_agent_identity();
    let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
    let (subject, _) = local_descriptor_subject(subject_ura)?;
    let runtime_ability = ability_ura_for_wire(callee_ura, ability)?;
    let descriptor_version =
        registered_descriptor_version(runtime, &runtime_ability, AxonInvocationCallMode::Bidi)
            .await?;
    let ability = ability_descriptor_ref_for_wire(callee_ura, ability, &descriptor_version)?;
    let descriptor_bound = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller,
        callee,
        ability,
        subject,
        invocation_nonce: fresh_nonce(),
        causal_context: CausalContext::None,
        args_bytes: &args,
    })?;
    let request = LocalRuntimeRequestFactory::request_for(
        AxonInvocationCallMode::Bidi,
        LocalRuntimeIngress::LocalSystem {
            envelope: descriptor_bound,
            payload: args,
        },
        LocalRuntimeRequestOptions::default(),
    )?;
    let (handle, _signed) = runtime
        .invoke_descriptor_bound_bidi_request_async(request)
        .await?;
    Ok(handle)
}

/// Daemon-internal dispatch. The shim binds execution to an explicit callee
/// and `EntityRef` subject, then signs with the synthetic `_system.local`
/// caller before entering Axon's public admission path.
pub async fn dispatch_rpc_local_with_subject(
    runtime: &Arc<LocalRuntime>,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: Vec<u8>,
) -> RpcDispatchOutcome {
    let caller = system_agent_identity();
    let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
    let (subject, _) = match local_descriptor_subject(subject_ura) {
        Ok(subject) => subject,
        Err(err) => {
            return RpcDispatchOutcome {
                invocation_id: None,
                state: InvocationState::Failed,
                payload_bytes: Vec::new(),
                error: Some(err),
                admission_receipt: None,
                terminal_receipt: None,
            };
        }
    };
    let runtime_ability = match ability_ura_for_wire(callee_ura, ability) {
        Ok(runtime_ability) => runtime_ability,
        Err(err) => {
            return RpcDispatchOutcome {
                invocation_id: None,
                state: InvocationState::Failed,
                payload_bytes: Vec::new(),
                error: Some(err),
                admission_receipt: None,
                terminal_receipt: None,
            };
        }
    };
    let descriptor_version =
        match registered_descriptor_version(runtime, &runtime_ability, AxonInvocationCallMode::Rpc)
            .await
        {
            Ok(descriptor_version) => descriptor_version,
            Err(err) => {
                return RpcDispatchOutcome {
                    invocation_id: None,
                    state: InvocationState::Failed,
                    payload_bytes: Vec::new(),
                    error: Some(err.into()),
                    admission_receipt: None,
                    terminal_receipt: None,
                };
            }
        };
    let ability = match ability_descriptor_ref_for_wire(callee_ura, ability, &descriptor_version) {
        Ok(ability) => ability,
        Err(err) => {
            return RpcDispatchOutcome {
                invocation_id: None,
                state: InvocationState::Failed,
                payload_bytes: Vec::new(),
                error: Some(err),
                admission_receipt: None,
                terminal_receipt: None,
            };
        }
    };
    let descriptor_bound = match DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller,
        callee,
        ability,
        subject,
        invocation_nonce: fresh_nonce(),
        causal_context: CausalContext::None,
        args_bytes: &args,
    }) {
        Ok(envelope) => envelope,
        Err(err) => {
            return RpcDispatchOutcome {
                invocation_id: None,
                state: InvocationState::Failed,
                payload_bytes: Vec::new(),
                error: Some(err),
                admission_receipt: None,
                terminal_receipt: None,
            };
        }
    };
    let request = match LocalRuntimeRequestFactory::request_for(
        AxonInvocationCallMode::Rpc,
        LocalRuntimeIngress::LocalSystem {
            envelope: descriptor_bound,
            payload: args,
        },
        LocalRuntimeRequestOptions::default(),
    ) {
        Ok(request) => request,
        Err(err) => {
            return RpcDispatchOutcome {
                invocation_id: None,
                state: InvocationState::Failed,
                payload_bytes: Vec::new(),
                error: Some(err),
                admission_receipt: None,
                terminal_receipt: None,
            };
        }
    };
    let result = runtime.invoke_descriptor_bound_request_async(request).await;
    match result {
        Ok((handle, _signed)) => drain_to_outcome(handle).await,
        Err(err) => RpcDispatchOutcome {
            invocation_id: None,
            state: InvocationState::Failed,
            payload_bytes: Vec::new(),
            error: Some(err),
            admission_receipt: None,
            terminal_receipt: None,
        },
    }
}

/// Project an `RpcDispatchOutcome` into the (payload, error_string)
/// pair the wire layer needs to build an
/// `InvokeRemoteDown::Result` frame.
///
/// `error_string` is `None` iff the dispatch completed cleanly
/// (`state == Completed`, `error == None`). Every operational
/// failure — `Failed` / `TimedOut` / `Cancelled` / pre-admission
/// rejection — produces `Some(<diagnostic>)` with the original
/// `AxonError`'s `Display` rendering. Caller wires this directly
/// into the in-band terminal frame helper at
/// `daemon::invocation::dispatch::daemon_invocation_service::
///  invoke_remote_inband_error_response` (already in place from
/// the earlier Phase-2 follow-up).
///
/// One canonical mapping site so the in-band-error wire shape
/// stays consistent across every dispatch site that flips to
/// Axon-routed dispatch.
#[must_use]
pub fn outcome_to_invoke_remote_result(outcome: RpcDispatchOutcome) -> (Vec<u8>, Option<String>) {
    let RpcDispatchOutcome {
        invocation_id: _,
        state: _,
        admission_receipt: _,
        terminal_receipt: _,
        payload_bytes,
        error,
    } = outcome;
    let error_string = error.map(|e| e.to_string());
    (payload_bytes, error_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    use easynet_axon::invocation::{
        fresh_nonce, make_ability, sha256, sign_descriptor_bound_invocation,
        signing_key_from_bytes, AbilityCallModes, AbilityOptions, AgentIdentity, AxonError,
        CausalContext, DescriptorBoundEnvelope, InvocationLedger, KeyResolver, LedgerSink,
        LocalRuntime, SubjectIdentity, UraProfile,
    };

    /// RPC options carrying the descriptor proof the receipt-proof
    /// normalizer requires, stamped at the same version the simulated wire
    /// envelope uses so `proof_descriptor_version` matches.
    fn wire_proof_bound_rpc_options() -> AbilityOptions {
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof(WIRE_TEST_DESCRIPTOR_VERSION, [0x11; 32], [0x22; 32])
    }

    fn production_proof_bound_rpc_options() -> AbilityOptions {
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof("1.0.0", [0x11; 32], [0x22; 32])
    }
    use base64::Engine;
    use ed25519_dalek::{SigningKey, VerifyingKey};
    use serde_json::Value;

    /// Descriptor version a simulated upstream client stamps onto the wire
    /// envelope. Deliberately not the production default constant: the
    /// inbound wire path must carry whatever version the caller declared,
    /// and the ledger must record exactly that.
    const WIRE_TEST_DESCRIPTOR_VERSION: &str = "3.1.4";

    struct FixedKey(VerifyingKey);
    impl KeyResolver for FixedKey {
        fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
            if agent_ura == crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA {
                return Ok(crate::daemon::identity::local_invocation::system_verifying_key());
            }
            Ok(self.0)
        }
    }

    fn build_test_runtime() -> (
        Arc<LocalRuntime>,
        Arc<InvocationLedger>,
        SigningKey,
        tempfile::TempDir,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
        let sk = signing_key_from_bytes(&[0x77; 32]);
        let rt = LocalRuntime::new();
        rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
        rt.set_admission_key_resolver(Arc::new(FixedKey(sk.verifying_key())));
        (rt, ledger, sk, temp)
    }

    fn build_wire_envelope(
        sk: &SigningKey,
        ability: &str,
        payload: &[u8],
    ) -> (pb::Envelope, String, Vec<u8>) {
        // Build the SDK envelope first (the source-of-truth for the
        // canonical bytes signed over), then project back into wire
        // form. This mirrors the shape an upstream client would have
        // produced.
        let caller_sdk =
            AgentIdentity::new("easynet:///r/t/agent/u.alice", UraProfile::EasynetStrictV2);
        let callee_sdk =
            AgentIdentity::new("easynet:///r/t/device/host", UraProfile::EasynetStrictV2);
        let subject_sdk = SubjectIdentity::from_callee(&callee_sdk);
        let nonce = fresh_nonce();
        let ability_ref =
            ability_descriptor_ref_for_wire(&callee_sdk.ura, ability, WIRE_TEST_DESCRIPTOR_VERSION)
                .unwrap();
        // Carry the versioned descriptor ref as the wire `function_name`
        // so reassembly preserves WIRE_TEST_DESCRIPTOR_VERSION instead of
        // defaulting — the same ref the signature is computed over.
        let wire_function_name = ability_ref.clone();
        let descriptor_bound_sdk =
            DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
                caller: caller_sdk.clone(),
                callee: callee_sdk.clone(),
                ability: ability_ref,
                subject: subject_sdk.clone(),
                invocation_nonce: nonce,
                causal_context: CausalContext::None,
                args_bytes: payload,
            })
            .unwrap();
        let sig_sdk = sign_descriptor_bound_invocation(sk, &descriptor_bound_sdk, "test-key");

        // Project the SDK pieces back into wire form. CLI's
        // outbound clients would do the inverse direction; for the
        // shim's inbound test we only need the wire field shapes.
        let wire_caller = pb::AgentIdentity {
            ura: caller_sdk.ura.clone(),
            profile: caller_sdk.profile.as_str().to_string(),
        };
        let wire_callee = pb::AgentIdentity {
            ura: callee_sdk.ura.clone(),
            profile: callee_sdk.profile.as_str().to_string(),
        };
        let wire_subject = pb::SubjectIdentity {
            ura: subject_sdk.ura.clone(),
            profile: subject_sdk.profile.as_str().to_string(),
        };
        let wire_sig = pb::CallerSignature {
            algorithm: sig_sdk.algorithm,
            signature: sig_sdk.signature,
            key_id_hint: sig_sdk.key_id_hint,
        };

        let wire_envelope = pb::Envelope {
            caller: Some(wire_caller),
            callee: Some(wire_callee),
            subject: Some(wire_subject),
            invocation_nonce: nonce.to_vec(),
            causal_context: Some(pb::CausalContext {
                form: Some(pb::causal_context::Form::None(pb::Empty {})),
            }),
            caller_signature: Some(wire_sig),
            ..Default::default()
        };
        let _ = sha256(payload); // assertion in test body
        (wire_envelope, wire_function_name, payload.to_vec())
    }

    #[tokio::test]
    async fn dispatch_rpc_completes_through_axon_runtime_and_persists() {
        let (rt, ledger, sk, _temp) = build_test_runtime();
        let callee_ura = "easynet:///r/t/device/host";
        let ability_ura = crate::core::ura::owner_ability_ura(callee_ura, "test.echo").unwrap();
        rt.register_ability_with_options(
            ability_ura.clone(),
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
            wire_proof_bound_rpc_options(),
        )
        .await
        .unwrap();

        let payload = serde_json::to_vec(&serde_json::json!({"x": 7})).unwrap();
        let (wire_env, ability, args) = build_wire_envelope(&sk, "test.echo", &payload);
        let dispatch = external_signed_from_wire_parts(wire_env, ability, args, Default::default())
            .expect("translate wire");

        let outcome = dispatch_rpc(&rt, dispatch).await;
        assert_eq!(outcome.state, InvocationState::Completed);
        assert!(outcome.error.is_none());
        assert!(
            outcome.invocation_id.is_some(),
            "terminal event id must be surfaced to the wire layer"
        );
        let echoed: Value = serde_json::from_slice(&outcome.payload_bytes).unwrap();
        assert_eq!(echoed, serde_json::json!({"x": 7}));

        // LedgerSink writes on the spawn task; give it a tick.
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let records = ledger.list_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].ability_name,
            format!("{ability_ura}@{WIRE_TEST_DESCRIPTOR_VERSION}")
        );
        assert_eq!(records[0].state, "completed");
        assert_eq!(records[0].caller_ura, "easynet:///r/t/agent/u.alice");
        assert_eq!(records[0].callee_ura, "easynet:///r/t/device/host");
        assert_eq!(records[0].subject_ura, "easynet:///r/t/device/host");
        assert_eq!(
            outcome.invocation_id.as_deref(),
            Some(records[0].request_id.as_str())
        );
    }

    #[tokio::test]
    async fn dispatch_rpc_accepts_backend_prepare_signed_fixture() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
        let public_key = base64::engine::general_purpose::STANDARD
            .decode("Ild93/uiXzP6DGBhe0FLETKB8GoOYz/QEKCl77wPYJ4=")
            .unwrap();
        let verifying_key =
            VerifyingKey::from_bytes(public_key.as_slice().try_into().unwrap()).unwrap();
        let rt = LocalRuntime::new();
        rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
        rt.set_admission_key_resolver(Arc::new(FixedKey(verifying_key)));

        let callee_ura = "easynet:///r/hub-a.local/device/be2146d3-2afe-4977-9f9a-245982b79db4";
        let ability_ura = crate::core::ura::owner_ability_ura(callee_ura, "shell.run").unwrap();
        rt.register_ability_with_options(
            ability_ura,
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
            production_proof_bound_rpc_options(),
        )
        .await
        .unwrap();

        let wire_env = pb::Envelope {
            caller: Some(pb::AgentIdentity {
                ura: "easynet:///r/hub-a.local/user/ad5a2619-4c49-459d-a862-a41111cc646d"
                    .to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(pb::AgentIdentity {
                ura: callee_ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(pb::SubjectIdentity {
                ura: "easynet:///r/hub-a.local/resource/user.ad5a2619-4c49-459d-a862-a41111cc646d/invoke/shell.run"
                    .to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: base64::engine::general_purpose::STANDARD
                .decode("70OZGnyx21TmUqWXuVfNag==")
                .unwrap(),
            causal_context: Some(pb::CausalContext {
                form: Some(pb::causal_context::Form::None(pb::Empty {})),
            }),
            caller_signature: Some(pb::CallerSignature {
                algorithm: "ed25519".to_string(),
                signature: base64::engine::general_purpose::STANDARD
                    .decode("s7GnQI8MVYQcxwkTX1dHoatVFVsLlFpn1O9LJQHZt8RElvZj3orC8jdcKKflvVVu93Ou5o9WWXEnRoYtlAZJAg==")
                    .unwrap(),
                key_id_hint: "Ild93/uiXzP6DGBhe0FLETKB8GoOYz/QEKCl77wPYJ4=".to_string(),
            }),
            ..Default::default()
        };
        let dispatch = external_signed_from_wire_parts(
            wire_env,
            "easynet:///r/hub-a.local/ability/device.be2146d3-2afe-4977-9f9a-245982b79db4.shell.run@1.0.0"
                .to_string(),
            br#"{"command":"hostname"}"#.to_vec(),
            Default::default(),
        )
        .expect("translate backend signed fixture");

        let outcome = dispatch_rpc(&rt, dispatch).await;
        assert_eq!(outcome.state, InvocationState::Completed);
        assert!(
            outcome.error.is_none(),
            "backend signed fixture must pass admission: {:?}",
            outcome.error
        );
    }

    #[tokio::test]
    async fn dispatch_rpc_unknown_ability_returns_in_band_error() {
        // Before Phase 4: this would have been Status::not_found
        // ("target not in PresenceRegistry") that the Go shim
        // logged as HTTP 500 with no body — the original bug the
        // user kept hitting. After Phase 4: the runtime answers
        // with a typed AxonError that the wire layer projects into
        // a structured response body.
        let (rt, _ledger, sk, _temp) = build_test_runtime();
        // No ability registered for "missing.thing".

        let payload = b"{}".to_vec();
        let (wire_env, ability, args) = build_wire_envelope(&sk, "missing.thing", &payload);
        let dispatch = external_signed_from_wire_parts(wire_env, ability, args, Default::default())
            .expect("translate wire");

        let outcome = dispatch_rpc(&rt, dispatch).await;
        // Axon rejects at the call-mode gate (which also implies
        // "unknown ability") BEFORE admission burns the nonce, so
        // the outcome surfaces the rejection without state
        // becoming Failed (the runtime never spawned the task).
        // Either way `error.is_some()` is the load-bearing
        // invariant for the wire-shape projection.
        assert!(
            outcome.error.is_some(),
            "unknown ability must surface a structured error, got {outcome:?}"
        );
        let err = outcome.error.unwrap();
        assert!(
            err.to_string().contains("unknown_ability")
                || err.to_string().contains("does not support"),
            "diagnostic must name the gate it failed: {err}"
        );
    }

    #[tokio::test]
    async fn external_signed_wire_rejects_missing_caller() {
        // Defensive: the daemon should never receive a wire
        // envelope without a caller, but
        // the shim must reject cleanly instead of panicking when
        // a malformed peer sends one anyway.
        let (_rt, _ledger, sk, _temp) = build_test_runtime();
        let (mut wire, ability, args) = build_wire_envelope(&sk, "x", b"{}");
        wire.caller = None;
        let err =
            external_signed_from_wire_parts(wire, ability, args, Default::default()).unwrap_err();
        assert!(err.to_string().contains("missing caller"));
    }

    #[tokio::test]
    async fn external_signed_wire_rejects_missing_signature() {
        let (_rt, _ledger, sk, _temp) = build_test_runtime();
        let (mut wire, ability, args) = build_wire_envelope(&sk, "x", b"{}");
        wire.caller_signature = None;
        let err =
            external_signed_from_wire_parts(wire, ability, args, Default::default()).unwrap_err();
        assert!(err.to_string().contains("missing caller_signature"));
    }

    #[tokio::test]
    async fn external_wire_rejects_missing_or_unprojectable_subject() {
        let base = || pb::Envelope {
            caller: Some(pb::AgentIdentity {
                ura: "easynet:///r/t/agent/u.x".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(pb::AgentIdentity {
                ura: "easynet:///r/t/device/h".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(pb::SubjectIdentity {
                ura: "easynet:///r/t/device/h".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: vec![0; 16],
            causal_context: Some(pb::CausalContext {
                form: Some(pb::causal_context::Form::None(pb::Empty {})),
            }),
            ..Default::default()
        };

        let mut missing_subject = base();
        missing_subject.subject = None;
        let err = external_signed_from_wire_parts(
            missing_subject,
            "x".to_string(),
            Vec::new(),
            Default::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing subject"));

        let mut unsupported_subject = base();
        unsupported_subject.subject = Some(pb::SubjectIdentity {
            ura: "easynet:///r/t/user/alice".to_string(),
            profile: "easynet-strict-v2".to_string(),
        });
        let err = external_signed_from_wire_parts(
            unsupported_subject,
            "x".to_string(),
            Vec::new(),
            Default::default(),
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("subject_ref_kind_unsupported:User"));
    }

    #[tokio::test]
    async fn dispatch_rpc_local_with_subject_runs_handler_with_system_signature() {
        let (rt, ledger, _sk, _temp) = build_test_runtime();
        let callee_ura = "easynet:///r/t/device/host";
        let ability_ura =
            crate::core::ura::owner_ability_ura(callee_ura, "demo.daemon_internal").unwrap();
        rt.register_ability_with_options(
            ability_ura.clone(),
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
            wire_proof_bound_rpc_options(),
        )
        .await
        .unwrap();

        let payload = b"\"hello-from-daemon\"".to_vec();
        let outcome = dispatch_rpc_local_with_subject(
            &rt,
            callee_ura,
            callee_ura,
            "demo.daemon_internal",
            payload.clone(),
        )
        .await;
        assert_eq!(outcome.state, InvocationState::Completed);
        assert!(outcome.error.is_none());
        assert!(
            outcome.invocation_id.is_some(),
            "terminal event id must be available for RPC response correlation"
        );
        assert_eq!(outcome.payload_bytes, payload);

        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let records = ledger.list_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].ability_name,
            format!("{ability_ura}@{WIRE_TEST_DESCRIPTOR_VERSION}")
        );
        assert_eq!(records[0].state, "completed");
        assert_eq!(
            outcome.invocation_id.as_deref(),
            Some(records[0].request_id.as_str())
        );
    }

    #[tokio::test]
    async fn dispatch_rpc_local_with_subject_unknown_ability_returns_in_band_error() {
        let (rt, _ledger, _sk, _temp) = build_test_runtime();
        let outcome = dispatch_rpc_local_with_subject(
            &rt,
            "easynet:///r/t/device/host",
            "easynet:///r/t/device/host",
            "no.such.thing",
            b"{}".to_vec(),
        )
        .await;
        assert!(outcome.error.is_some());
        let err = outcome.error.unwrap();
        assert!(
            err.to_string().contains("unknown_ability")
                || err.to_string().contains("does not support"),
            "diagnostic must name the gate: {err}"
        );
    }

    #[tokio::test]
    async fn outcome_to_invoke_remote_result_completed_drops_error() {
        let outcome = RpcDispatchOutcome {
            invocation_id: None,
            state: InvocationState::Completed,
            payload_bytes: b"ok".to_vec(),
            error: None,
            admission_receipt: None,
            terminal_receipt: None,
        };
        let (payload, err) = outcome_to_invoke_remote_result(outcome);
        assert_eq!(payload, b"ok");
        assert!(err.is_none());
    }

    #[tokio::test]
    async fn outcome_to_invoke_remote_result_failed_carries_diagnostic_string() {
        let outcome = RpcDispatchOutcome {
            invocation_id: None,
            state: InvocationState::Failed,
            payload_bytes: Vec::new(),
            error: Some(AxonError::internal("synthetic boom")),
            admission_receipt: None,
            terminal_receipt: None,
        };
        let (payload, err) = outcome_to_invoke_remote_result(outcome);
        assert!(payload.is_empty());
        let msg = err.expect("failed outcome must carry an error string");
        assert!(
            msg.contains("synthetic boom"),
            "diagnostic must round-trip: {msg}"
        );
    }

    #[tokio::test]
    async fn external_signed_wire_rejects_missing_subject() {
        let (_rt, _ledger, sk, _temp) = build_test_runtime();
        let payload = b"{}".to_vec();
        let (mut wire_env, ability, args) = build_wire_envelope(&sk, "any", &payload);
        wire_env.subject = None;
        let err = external_signed_from_wire_parts(wire_env, ability, args, Default::default())
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid_argument reason=wire envelope missing subject"
        );
    }
}
