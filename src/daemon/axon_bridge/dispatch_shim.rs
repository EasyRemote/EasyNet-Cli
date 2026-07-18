//! LocalRuntime dispatch adapter: route daemon ingress through Axon's
//! public descriptor-bound APIs.
//!
//! Goal
//! ----
//! Keep one descriptor-bound execution path for every daemon ingress:
//!
//!   1. Take the already-extracted descriptor-bound wire pieces.
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

use axon_sdk::invocation::{
    AxonError, BidiInvocationHandle, CallMode as AxonInvocationCallMode, CallerSignature,
    CausalContext, DescriptorBoundEnvelope, DescriptorBoundInvocationRequest, InvocationHandle,
    InvocationState, LocalRuntime, StreamingInvocationHandle,
};
use axon_sdk::pb::axon::v1 as pb;

use crate::daemon::axon_bridge::descriptor_ref::{
    ability_descriptor_ref_for_wire, ability_ura_for_wire, registered_descriptor_binding,
};
use crate::daemon::axon_bridge::local_runtime_request::{
    LocalRuntimeIngress, LocalRuntimeRequestFactory, LocalRuntimeRequestOptions,
    SystemInvocationIssuer,
};
use crate::daemon::axon_bridge::wire_descriptor::descriptor_bound_from_wire_parts;

/// Explicit admission class for a wire-shaped LocalRuntime dispatch.
#[derive(Debug)]
pub enum WireDispatchIngress {
    /// Caller-supplied signature that Axon must verify.
    ExternalSigned(CallerSignature),
    /// First-join signature whose candidate key is leased into the CLI
    /// admission resolver for canonical Axon verification.
    ProvisionalBootstrap(CallerSignature),
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
    local_system_authority: Option<LocalSystemAuthority>,
    provisional_key_lease:
        Option<crate::daemon::axon_bridge::runtime_admin::ProvisionalBootstrapKeyLease>,
}

#[derive(Debug)]
struct LocalSystemAuthority;

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
    let reassembled =
        descriptor_bound_from_wire_parts(envelope.clone(), target_ability_name, &initial_args)
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
        local_system_authority: matches!(policy, WireIngressPolicy::LocalSystem)
            .then_some(LocalSystemAuthority),
        provisional_key_lease: None,
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
pub(crate) fn local_system_from_wire_parts(
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

pub(crate) fn provisional_bootstrap_from_wire_parts(
    envelope: pb::Envelope,
    target_ability_name: String,
    initial_args: Vec<u8>,
    request_metadata: HashMap<String, String>,
    public_key: [u8; 32],
    key_provider: &Arc<crate::daemon::axon_bridge::runtime_admin::ProvisionalBootstrapKeyProvider>,
) -> Result<WireDispatch, Box<AxonError>> {
    let reassembled =
        descriptor_bound_from_wire_parts(envelope.clone(), target_ability_name, &initial_args)
            .map_err(Box::new)?;
    let signature = envelope
        .caller_signature
        .clone()
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing caller_signature"))?
        .into();
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&public_key).map_err(|error| {
        Box::new(AxonError::invalid_argument(format!(
            "provisional_bootstrap_public_key:{error}"
        )))
    })?;
    let provisional_key_lease = key_provider
        .lease_candidate(&reassembled.envelope.envelope().caller.ura, verifying_key)
        .map_err(Box::new)?;
    Ok(WireDispatch {
        envelope: reassembled.envelope,
        ingress: WireDispatchIngress::ProvisionalBootstrap(signature),
        payload: initial_args,
        request_metadata,
        trace_id: reassembled.trace_id,
        local_system_authority: None,
        provisional_key_lease: Some(provisional_key_lease),
    })
}

struct PreparedWireDispatch {
    request: DescriptorBoundInvocationRequest,
    _provisional_key_lease:
        Option<crate::daemon::axon_bridge::runtime_admin::ProvisionalBootstrapKeyLease>,
}

fn request_for_wire_dispatch(
    mode: AxonInvocationCallMode,
    wire: WireDispatch,
) -> Result<PreparedWireDispatch, AxonError> {
    let WireDispatch {
        envelope,
        ingress,
        payload,
        request_metadata,
        trace_id,
        local_system_authority,
        provisional_key_lease,
    } = wire;
    let ingress = match ingress {
        WireDispatchIngress::ExternalSigned(signature) => LocalRuntimeIngress::ExternalSigned {
            envelope,
            signature,
            payload,
        },
        WireDispatchIngress::ProvisionalBootstrap(signature) => {
            LocalRuntimeIngress::ExternalSigned {
                envelope,
                signature,
                payload,
            }
        }
        WireDispatchIngress::LocalSystem => {
            local_system_authority.ok_or_else(|| {
                AxonError::permission_denied(
                    "local system dispatch requires trusted-local transport authority",
                )
            })?;
            let request = SystemInvocationIssuer::request_for_complete_envelope(
                mode,
                envelope,
                payload,
                LocalRuntimeRequestOptions::default()
                    .with_trace_id(trace_id)
                    .with_request_metadata(request_metadata),
            )?;
            return Ok(PreparedWireDispatch {
                request,
                _provisional_key_lease: provisional_key_lease,
            });
        }
    };
    let request = LocalRuntimeRequestFactory::request_for(
        mode,
        ingress,
        LocalRuntimeRequestOptions::default()
            .with_trace_id(trace_id)
            .with_request_metadata(request_metadata),
    )?;
    Ok(PreparedWireDispatch {
        request,
        _provisional_key_lease: provisional_key_lease,
    })
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
    pub admission_receipt: Option<axon_sdk::invocation::SignedInvocationReceipt>,
    /// Terminal execution receipt, when the runtime minted one —
    /// carried back to the hub on carrier-v1 sessions (DEC-F004).
    pub terminal_receipt: Option<axon_sdk::invocation::SignedInvocationReceipt>,
}

/// Drain an `InvocationHandle` through Axon's canonical finalization view.
/// Receipt selection, terminal-state validation and typed failure recovery all
/// remain owned by the runtime; this adapter only projects the immutable result.
async fn drain_to_outcome(handle: InvocationHandle) -> RpcDispatchOutcome {
    let invocation_id = Some(handle.invocation_id().to_string());
    match handle.finalized().await {
        Ok(finalized) => {
            let completed = finalized.terminal_state == InvocationState::Completed;
            RpcDispatchOutcome {
                invocation_id,
                state: finalized.terminal_state,
                payload_bytes: if completed {
                    finalized.output().to_vec()
                } else {
                    Vec::new()
                },
                error: finalized.failure,
                admission_receipt: Some(finalized.admission_receipt),
                terminal_receipt: Some(finalized.terminal_receipt),
            }
        }
        Err(error) => RpcDispatchOutcome {
            invocation_id,
            state: handle.current_state().await,
            payload_bytes: Vec::new(),
            error: Some(error),
            admission_receipt: None,
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
async fn dispatch_rpc(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
    cancellations: &crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
) -> RpcDispatchOutcome {
    let lifecycle_envelope = wire.envelope.clone();
    let prepared = match request_for_wire_dispatch(AxonInvocationCallMode::Rpc, wire) {
        Ok(prepared) => prepared,
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
    let result = runtime
        .invoke_descriptor_bound_request_async(prepared.request)
        .await;
    match result {
        Ok((handle, _signed)) => {
            let lifecycle_key = match cancellations.register(&lifecycle_envelope, handle.clone()) {
                Ok(key) => key,
                Err(err) => return cancellation_error_outcome(err),
            };
            let outcome = drain_to_outcome(handle.clone()).await;
            if outcome.terminal_receipt.is_some() {
                cancellations.mark_terminal(&lifecycle_key, handle);
            }
            outcome
        }
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

fn cancellation_error_outcome(
    error: crate::daemon::invocation::dispatch::cancellation::InvocationCancellationError,
) -> RpcDispatchOutcome {
    RpcDispatchOutcome {
        invocation_id: None,
        state: InvocationState::Failed,
        payload_bytes: Vec::new(),
        error: Some(AxonError::unavailable(format!(
            "invocation_cancel_request_failed:{error}"
        ))),
        admission_receipt: None,
        terminal_receipt: None,
    }
}

/// Unary entry for daemon-admitted externally signed callers.
pub(crate) async fn dispatch_rpc_admitted(
    runtime: &Arc<LocalRuntime>,
    wire: WireDispatch,
    cancellations: &crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
) -> RpcDispatchOutcome {
    dispatch_rpc(runtime, wire, cancellations).await
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
    let prepared = request_for_wire_dispatch(AxonInvocationCallMode::Stream, wire)?;
    let (handle, _signed) = runtime
        .invoke_descriptor_bound_stream_request_async(prepared.request)
        .await?;
    Ok(handle)
}

async fn local_system_descriptor_ref(
    runtime: &Arc<LocalRuntime>,
    callee_ura: &str,
    ability: &str,
    mode: AxonInvocationCallMode,
) -> Result<String, AxonError> {
    let runtime_ability = ability_ura_for_wire(callee_ura, ability)?;
    let descriptor_binding = registered_descriptor_binding(runtime, &runtime_ability, mode).await?;
    ability_descriptor_ref_for_wire(callee_ura, ability, &descriptor_binding)
}

pub async fn open_stream_local_with_subject(
    runtime: &Arc<LocalRuntime>,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: Vec<u8>,
) -> Result<StreamingInvocationHandle, AxonError> {
    let ability =
        local_system_descriptor_ref(runtime, callee_ura, ability, AxonInvocationCallMode::Stream)
            .await?;
    let request = SystemInvocationIssuer::request_for_descriptor_ref(
        AxonInvocationCallMode::Stream,
        callee_ura,
        ability,
        subject_ura,
        args,
        CausalContext::None,
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
    let prepared = request_for_wire_dispatch(AxonInvocationCallMode::Bidi, wire)?;
    let (handle, _signed) = runtime
        .invoke_descriptor_bound_bidi_request_async(prepared.request)
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
    let ability =
        local_system_descriptor_ref(runtime, callee_ura, ability, AxonInvocationCallMode::Bidi)
            .await?;
    let request = SystemInvocationIssuer::request_for_descriptor_ref(
        AxonInvocationCallMode::Bidi,
        callee_ura,
        ability,
        subject_ura,
        args,
        CausalContext::None,
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
    let ability = match local_system_descriptor_ref(
        runtime,
        callee_ura,
        ability,
        AxonInvocationCallMode::Rpc,
    )
    .await
    {
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
    let request = match SystemInvocationIssuer::request_for_descriptor_ref(
        AxonInvocationCallMode::Rpc,
        callee_ura,
        ability,
        subject_ura,
        args,
        CausalContext::None,
        LocalRuntimeRequestOptions::default(),
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire;

    use axon_sdk::invocation::{
        fresh_nonce, make_ability, sha256, sign_descriptor_bound_invocation,
        signing_key_from_bytes, AbilityCallModes, AbilityOptions, AgentIdentity, AxonError,
        CausalContext, DescriptorBoundEnvelope, DescriptorBoundEnvelopeParts, InvocationLedger,
        KeyResolver, LocalRuntime, SubjectIdentity, UraProfile,
    };

    /// RPC options carrying the descriptor proof the receipt-proof
    /// normalizer requires, stamped at the same version the simulated wire
    /// envelope uses so `proof_descriptor_version` matches.
    fn wire_proof_bound_rpc_options() -> AbilityOptions {
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof(
                WIRE_TEST_DESCRIPTOR_VERSION,
                "invoke",
                [0x33; 32],
                [0x11; 32],
                [0x22; 32],
            )
    }

    fn production_proof_bound_rpc_options() -> AbilityOptions {
        AbilityOptions::default()
            .with_modes(AbilityCallModes::RPC)
            .with_descriptor_proof("1.0.0", "invoke", [0x33; 32], [0x11; 32], [0x22; 32])
    }
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
                return crate::daemon::identity::local_invocation::system_verifying_key()
                    .map_err(|error| AxonError::internal(error.to_string()));
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
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            Arc::new(FixedKey(sk.verifying_key())),
            Some(Arc::clone(&ledger)),
        );
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
        let caller_sdk = AgentIdentity::new("easynet:///r/t/agent/u.alice", UraProfile::StrictV2);
        let callee_sdk = AgentIdentity::new("easynet:///r/t/device/host", UraProfile::StrictV2);
        let subject_sdk = SubjectIdentity::from_callee(&callee_sdk);
        let nonce = fresh_nonce();
        let ability_ref = ability_descriptor_ref_for_wire(
            &callee_sdk.ura,
            ability,
            &descriptor_binding_for_wire(WIRE_TEST_DESCRIPTOR_VERSION, [0x33; 32], "invoke")
                .unwrap(),
        )
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

    fn wire_test_descriptor_binding() -> String {
        descriptor_binding_for_wire(WIRE_TEST_DESCRIPTOR_VERSION, [0x33; 32], "invoke").unwrap()
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

        let outcome = dispatch_rpc(&rt, dispatch, &Default::default()).await;
        assert_eq!(
            outcome.state,
            InvocationState::Completed,
            "dispatch failed: {:?}",
            outcome.error
        );
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
            format!("{ability_ura}@{}", wire_test_descriptor_binding())
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
    async fn cancellation_requires_target_owner_and_target_receipt_stays_canonical() {
        use crate::daemon::invocation::dispatch::cancellation::{
            invocation_lifecycle_hash, InvocationCancelCommand, InvocationCancellationError,
            InvocationCancellationRegistry,
        };

        let (runtime, _ledger, signing_key, _temp) = build_test_runtime();
        let callee_ura = "easynet:///r/t/device/host";
        let ability_ura = crate::core::ura::owner_ability_ura(callee_ura, "test.pending").unwrap();
        runtime
            .register_ability_with_options(
                ability_ura,
                make_ability(|_| async move {
                    std::future::pending::<Result<Vec<u8>, AxonError>>().await
                }),
                wire_proof_bound_rpc_options(),
            )
            .await
            .unwrap();

        let (wire_envelope, ability, args) =
            build_wire_envelope(&signing_key, "test.pending", b"{}");
        let dispatch =
            external_signed_from_wire_parts(wire_envelope, ability, args, Default::default())
                .expect("translate wire");
        let lifecycle_hash = invocation_lifecycle_hash(&dispatch.envelope);
        let cancellations = InvocationCancellationRegistry::default();
        let dispatch_task = {
            let runtime = Arc::clone(&runtime);
            let cancellations = cancellations.clone();
            tokio::spawn(async move { dispatch_rpc(&runtime, dispatch, &cancellations).await })
        };

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !cancellations.contains(&lifecycle_hash) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("target lifecycle is registered before cancellation");

        let command = InvocationCancelCommand::new(&lifecycle_hash, None, "operator stop")
            .expect("valid cancel command");
        let denied = cancellations
            .request_cancel(
                command.clone(),
                "easynet:///r/t/agent/u.mallory",
                callee_ura,
            )
            .await
            .expect_err("a different caller cannot cancel the target");
        assert!(matches!(
            denied,
            InvocationCancellationError::OwnershipDenied
        ));

        let accepted = cancellations
            .request_cancel(command, "easynet:///r/t/agent/u.alice", callee_ura)
            .await
            .expect("target owner can request cancellation");
        assert!(accepted.accepted);
        assert!(!accepted.already_terminal);

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), dispatch_task)
            .await
            .expect("cancelled target reaches terminal finalization")
            .expect("dispatch task joins");
        assert_eq!(outcome.state, InvocationState::Cancelled);
        let target_id = outcome
            .invocation_id
            .as_deref()
            .expect("target invocation id");
        assert_eq!(accepted.target_invocation_id, target_id);
        assert_eq!(
            outcome
                .terminal_receipt
                .as_ref()
                .map(|receipt| receipt.invocation_id()),
            Some(target_id)
        );
        assert!(outcome.admission_receipt.is_some());
    }

    #[tokio::test]
    async fn dispatch_rpc_accepts_backend_prepare_signed_fixture() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = Arc::new(InvocationLedger::open(temp.path().join("inv.redb")).unwrap());
        let signing_key = signing_key_from_bytes(&[0x24; 32]);
        let verifying_key = signing_key.verifying_key();
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            Arc::new(FixedKey(verifying_key)),
            Some(Arc::clone(&ledger)),
        );

        let callee_ura = "easynet:///r/hub-a.local/device/be2146d3-2afe-4977-9f9a-245982b79db4";
        let ability_ura = crate::core::ura::owner_ability_ura(callee_ura, "shell.run").unwrap();
        rt.register_ability_with_options(
            ability_ura,
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
            production_proof_bound_rpc_options(),
        )
        .await
        .unwrap();

        let caller_sdk = AgentIdentity::new(
            "easynet:///r/hub-a.local/user/ad5a2619-4c49-459d-a862-a41111cc646d",
            UraProfile::StrictV2,
        );
        let callee_sdk = AgentIdentity::new(callee_ura, UraProfile::StrictV2);
        let subject_sdk = SubjectIdentity::new(
            "easynet:///r/hub-a.local/resource/user.ad5a2619-4c49-459d-a862-a41111cc646d/invoke/shell.run",
            UraProfile::StrictV2,
        );
        let ability_ref = ability_descriptor_ref_for_wire(
            &callee_sdk.ura,
            "shell.run",
            &descriptor_binding_for_wire("1.0.0", [0x33; 32], "invoke").unwrap(),
        )
        .expect("canonical backend descriptor ref");
        let nonce = [0x24; 16];
        let descriptor_bound = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
            caller: caller_sdk.clone(),
            callee: callee_sdk.clone(),
            ability: ability_ref.clone(),
            subject: subject_sdk.clone(),
            invocation_nonce: nonce,
            causal_context: CausalContext::None,
            args_bytes: br#"{"command":"hostname"}"#,
        })
        .expect("backend descriptor-bound fixture");
        let signature =
            sign_descriptor_bound_invocation(&signing_key, &descriptor_bound, "test-key");
        let wire_env = pb::Envelope {
            caller: Some(pb::AgentIdentity {
                ura: caller_sdk.ura,
                profile: caller_sdk.profile.as_str().to_string(),
            }),
            callee: Some(pb::AgentIdentity {
                ura: callee_sdk.ura,
                profile: callee_sdk.profile.as_str().to_string(),
            }),
            subject: Some(pb::SubjectIdentity {
                ura: subject_sdk.ura,
                profile: subject_sdk.profile.as_str().to_string(),
            }),
            invocation_nonce: nonce.to_vec(),
            causal_context: Some(pb::CausalContext {
                form: Some(pb::causal_context::Form::None(pb::Empty {})),
            }),
            caller_signature: Some(pb::CallerSignature {
                algorithm: signature.algorithm,
                signature: signature.signature,
                key_id_hint: signature.key_id_hint,
            }),
            ..Default::default()
        };
        let dispatch = external_signed_from_wire_parts(
            wire_env,
            ability_ref,
            br#"{"command":"hostname"}"#.to_vec(),
            Default::default(),
        )
        .expect("translate backend signed fixture");

        let outcome = dispatch_rpc(&rt, dispatch, &Default::default()).await;
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

        let outcome = dispatch_rpc(&rt, dispatch, &Default::default()).await;
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

    #[test]
    fn local_system_request_rejects_an_unsealed_dispatch() {
        let (_rt, _ledger, signing_key, _temp) = build_test_runtime();
        let (wire, ability, args) = build_wire_envelope(&signing_key, "x", b"{}");
        let mut dispatch = external_signed_from_wire_parts(wire, ability, args, Default::default())
            .expect("complete descriptor-bound wire dispatch");
        dispatch.ingress = WireDispatchIngress::LocalSystem;
        dispatch.local_system_authority = None;

        let error = match request_for_wire_dispatch(AxonInvocationCallMode::Rpc, dispatch) {
            Ok(_) => panic!("an unsealed local-system dispatch must fail"),
            Err(error) => error,
        };
        assert!(error
            .to_string()
            .contains("requires trusted-local transport authority"));
    }

    #[test]
    fn local_system_request_rejects_a_non_system_caller() {
        let (_rt, _ledger, signing_key, _temp) = build_test_runtime();
        let (wire, ability, args) = build_wire_envelope(&signing_key, "x", b"{}");
        let dispatch = local_system_from_wire_parts(wire, ability, args, Default::default())
            .expect("complete trusted-local wire dispatch");

        let error = match request_for_wire_dispatch(AxonInvocationCallMode::Rpc, dispatch) {
            Ok(_) => panic!("trusted-local classification must not replace caller identity"),
            Err(error) => error,
        };
        assert!(error
            .to_string()
            .contains("local system ingress caller must be"));
    }

    #[tokio::test]
    async fn external_wire_rejects_missing_or_unprojectable_subject() {
        let callee_ura = "easynet:///r/t/device/h";
        let ability = ability_descriptor_ref_for_wire(
            callee_ura,
            "x",
            &descriptor_binding_for_wire(WIRE_TEST_DESCRIPTOR_VERSION, [0x33; 32], "invoke")
                .expect("descriptor binding"),
        )
        .expect("descriptor ref");
        let base = || pb::Envelope {
            caller: Some(pb::AgentIdentity {
                ura: "easynet:///r/t/agent/u.x".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(pb::AgentIdentity {
                ura: callee_ura.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(pb::SubjectIdentity {
                ura: callee_ura.to_string(),
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
            ability.clone(),
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
            ability,
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
        assert_eq!(
            outcome.state,
            InvocationState::Completed,
            "dispatch failed: {:?}",
            outcome.error
        );
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
            format!("{ability_ura}@{}", wire_test_descriptor_binding())
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
