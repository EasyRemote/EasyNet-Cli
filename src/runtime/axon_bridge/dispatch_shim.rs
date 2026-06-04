//! Phase-4 dispatch shim: route wire envelopes through Axon's
//! `LocalRuntime` callee-side APIs.
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
//!   3. Hand the (envelope, optional signature, args) triple to
//!      `LocalRuntime::invoke_admitted_*_async` when the daemon
//!      admission facade has already verified the caller, or to
//!      `LocalRuntime::invoke_externally_signed_*_async` when this
//!      shim owns admission for that call site.
//!
//! After Phase 4: there is no longer a "self-target shortcut" that
//! looks different from a "remote-target" path inside the daemon.
//! Both go through `LocalRuntime`; the only thing that varies is
//! whether the ability's handler runs in-process (because it was
//! registered) or the runtime returns
//! `AxonError::invalid_argument("unknown_ability:...")` (the
//! universal "I don't host this" signal).

use std::sync::Arc;

use easynet_axon::invocation::{
    fresh_nonce, AgentIdentity, AxonError, BidiInvocationHandle, CallerSignature, CausalContext,
    InvocationEnvelope, InvocationHandle, InvocationState, LocalRuntime, StreamingInvocationHandle,
    SubjectIdentity, UraProfile,
};

use easynet_axon::invocation::wire;
use easynet_axon::pb::axon::v1 as pb;

/// Canonical wire dispatch for call sites where this shim owns strict
/// admission. The caller signature is mandatory because the next step is
/// `invoke_externally_signed_*`.
#[derive(Debug)]
pub struct WireDispatch {
    pub envelope: InvocationEnvelope,
    pub signature: CallerSignature,
    pub payload: Vec<u8>,
}

/// Wire dispatch after the daemon's transport admission facade has already
/// admitted the caller. The optional signature is retained for audit/context
/// storage, but this path does not re-run admission or nonce replay.
#[derive(Debug)]
pub struct AdmittedWireDispatch {
    pub envelope: InvocationEnvelope,
    pub signature: Option<CallerSignature>,
    pub payload: Vec<u8>,
}

/// Reassemble (envelope, signature, payload) from the wire
/// EnvelopeOpen + InvocationTarget pair carried on bidi frame 0
/// (or projected from a unary InvokeRequest by the wrapping layer).
///
/// Encodes the design rule from
/// `core/proto/axon/v1/types.proto:429-446`: the wire `Envelope`
/// carries the 4 binding fields; `ability` and `args` live on the
/// surrounding shape. We thread both halves into `from_wire_parts`
/// so the canonical 7-tuple is reassembled in one place — the
/// load-bearing invariant being that `sha256(payload)` matches the
/// digest the caller signed over.
pub fn from_envelope_open(
    envelope: pb::Envelope,
    target_ability_name: String,
    initial_args: Vec<u8>,
) -> Result<WireDispatch, AxonError> {
    let signature = envelope
        .caller_signature
        .clone()
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing caller_signature"))?;
    let admitted = admitted_from_wire_parts(envelope, target_ability_name, initial_args)?;
    Ok(WireDispatch {
        envelope: admitted.envelope,
        signature: signature.into(),
        payload: admitted.payload,
    })
}

/// Reassemble a wire envelope after admission has already succeeded.
///
/// Unlike [`from_envelope_open`], this accepts unsigned Device/loopback
/// envelopes because the caller admission decision was made by
/// `AdmissionFacade` at the gRPC boundary. It still builds the canonical
/// Axon envelope so LocalRuntime handlers see caller/callee/subject.
pub fn admitted_from_wire_parts(
    envelope: pb::Envelope,
    target_ability_name: String,
    initial_args: Vec<u8>,
) -> Result<AdmittedWireDispatch, AxonError> {
    let caller = envelope
        .caller
        .ok_or_else(|| AxonError::invalid_argument("wire envelope missing caller"))?;
    let callee = envelope.callee.unwrap_or_else(|| caller.clone());
    let signature = envelope.caller_signature.map(Into::into);

    let caller_sdk = wire::try_agent_identity_from_wire(caller)?;
    let callee_sdk = wire::try_agent_identity_from_wire(callee)?;
    // Subject defaulting per RFC 001 §5.1: if the wire omits it,
    // mirror callee. Doing this at the boundary keeps every
    // downstream consumer (admission, audit, ledger) seeing a
    // populated subject without sprinkling `if subject.is_none()`
    // through the call graph.
    let subject_sdk: SubjectIdentity = match envelope.subject {
        Some(subject) => wire::try_subject_identity_from_wire(subject)?,
        None => SubjectIdentity::from_callee(&callee_sdk),
    };

    let nonce = wire::try_invocation_nonce(envelope.invocation_nonce)?;
    let causal_context = wire::causal_context_from_wire(envelope.causal_context)?;

    let envelope_sdk = InvocationEnvelope::from_wire_parts(
        caller_sdk,
        callee_sdk,
        subject_sdk,
        nonce,
        causal_context,
        target_ability_name,
        &initial_args,
    );

    Ok(AdmittedWireDispatch {
        envelope: envelope_sdk,
        signature,
        payload: initial_args,
    })
}

pub fn admitted_from_envelope_open(
    envelope_open: &pb::EnvelopeOpen,
) -> Result<AdmittedWireDispatch, AxonError> {
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
    admitted_from_wire_parts(envelope, ability, envelope_open.initial_args.clone())
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

    match (state, terminal) {
        (InvocationState::Completed, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id),
            state,
            payload_bytes: ev.payload,
            error: None,
        },
        (InvocationState::Failed, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id.clone()),
            state,
            payload_bytes: Vec::new(),
            error: Some(
                AxonError::internal(ev.reason.clone()).with_invocation_id(ev.invocation_id),
            ),
        },
        (InvocationState::TimedOut, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id.clone()),
            state,
            payload_bytes: Vec::new(),
            error: Some(
                AxonError::deadline_exceeded(ev.reason.clone())
                    .with_invocation_id(ev.invocation_id),
            ),
        },
        (InvocationState::Cancelled, Some(ev)) => RpcDispatchOutcome {
            invocation_id: Some(ev.invocation_id.clone()),
            state,
            payload_bytes: Vec::new(),
            error: Some(
                AxonError::cancelled(ev.reason.clone()).with_invocation_id(ev.invocation_id),
            ),
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
    let WireDispatch {
        envelope,
        signature,
        payload,
    } = wire;
    match runtime
        .invoke_externally_signed_async(envelope, signature, payload, None, None)
        .await
    {
        Ok((handle, _signed)) => drain_to_outcome(handle).await,
        Err(err) => RpcDispatchOutcome {
            invocation_id: None,
            state: InvocationState::Failed,
            payload_bytes: Vec::new(),
            error: Some(err),
        },
    }
}

/// Unary entry for callers already admitted by the daemon transport layer.
pub async fn dispatch_rpc_admitted(
    runtime: &Arc<LocalRuntime>,
    wire: AdmittedWireDispatch,
) -> RpcDispatchOutcome {
    let AdmittedWireDispatch {
        envelope,
        signature,
        payload,
    } = wire;
    match runtime
        .invoke_admitted_async(envelope, signature, payload, None, None)
        .await
    {
        Ok((handle, _signed)) => drain_to_outcome(handle).await,
        Err(err) => RpcDispatchOutcome {
            invocation_id: None,
            state: InvocationState::Failed,
            payload_bytes: Vec::new(),
            error: Some(err),
        },
    }
}

pub async fn open_stream_admitted(
    runtime: &Arc<LocalRuntime>,
    wire: AdmittedWireDispatch,
) -> Result<StreamingInvocationHandle, AxonError> {
    let AdmittedWireDispatch {
        envelope,
        signature,
        payload,
    } = wire;
    let (handle, _signed) = runtime
        .invoke_admitted_stream_async(envelope, signature, payload, None, None)
        .await?;
    Ok(handle)
}

pub async fn open_stream_local(
    runtime: &Arc<LocalRuntime>,
    ability: &str,
    args: Vec<u8>,
) -> Result<StreamingInvocationHandle, AxonError> {
    runtime.invoke_stream_async(ability, args, None, None).await
}

pub async fn open_stream_local_with_subject(
    runtime: &Arc<LocalRuntime>,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: Vec<u8>,
) -> Result<StreamingInvocationHandle, AxonError> {
    let caller = AgentIdentity::new(
        "easynet:///r/_system/agent/_system.local",
        UraProfile::EasynetStrictV2,
    );
    let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
    let subject = SubjectIdentity::new(subject_ura.to_string(), UraProfile::EasynetStrictV2);
    let envelope = InvocationEnvelope::from_wire_parts(
        caller,
        callee,
        subject,
        fresh_nonce(),
        CausalContext::None,
        ability,
        &args,
    );
    open_stream_admitted(
        runtime,
        AdmittedWireDispatch {
            envelope,
            signature: None,
            payload: args,
        },
    )
    .await
}

pub async fn open_bidi_admitted(
    runtime: &Arc<LocalRuntime>,
    wire: AdmittedWireDispatch,
) -> Result<BidiInvocationHandle, AxonError> {
    let AdmittedWireDispatch {
        envelope,
        signature,
        payload,
    } = wire;
    let (handle, _signed) = runtime
        .invoke_admitted_bidi_async(envelope, signature, payload, None, None)
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
    let caller = AgentIdentity::new(
        "easynet:///r/_system/agent/_system.local",
        UraProfile::EasynetStrictV2,
    );
    let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
    let subject = SubjectIdentity::new(subject_ura.to_string(), UraProfile::EasynetStrictV2);
    let envelope = InvocationEnvelope::from_wire_parts(
        caller,
        callee,
        subject,
        fresh_nonce(),
        CausalContext::None,
        ability,
        &args,
    );
    open_bidi_admitted(
        runtime,
        AdmittedWireDispatch {
            envelope,
            signature: None,
            payload: args,
        },
    )
    .await
}

/// Daemon-internal self-target dispatch — caller is trusted, no
/// signature verification.
///
/// This is the path the existing `<self>.invoke_remote` uses for
/// dispatching an inner ability whose admission was already done at
/// the outer boundary (backend authenticated the user, decomposed
/// the request, and re-issued it as a daemon-internal hub→agent
/// call). The Axon SDK's `invoke_async` runs with a SystemAgent
/// binding (AXIOM §3.2) — axiom-bound at the bytes level but
/// not cryptographically signed.
///
/// Why this exists alongside [`dispatch_rpc`]: the daemon serves
/// two kinds of self-target dispatch right now:
///   * **End-to-end signed** (`dispatch_rpc`): the caller's
///     ed25519 signature flows through, admission runs, ledger
///     records the full receipt chain with the real caller URA.
///     This is the right path when the wire shape carries the
///     user's envelope intact (future: Pages frontend → daemon
///     direct, bypassing the Go shim's re-issue).
///   * **Trust-domain dispatch** (`dispatch_rpc_local`): the
///     daemon's caller (the Go backend / a hub peer / itself) is
///     already trusted; admission is a no-op, the ledger records
///     a SystemAgent-bound row. This is the only path that
///     currently keeps the legacy InvokeRemoteUp wire shape
///     working — `<self>.invoke_remote` doesn't carry an inner
///     user signature, so end-to-end mode is impossible here
///     until the wire shape evolves.
///
/// Both paths feed the same LedgerSink installed at boot, so the
/// audit trail is uniform; they only differ in caller identity
/// strength.
pub async fn dispatch_rpc_local(
    runtime: &Arc<LocalRuntime>,
    ability: &str,
    args: Vec<u8>,
) -> RpcDispatchOutcome {
    match runtime.invoke_async(ability, args, None, None).await {
        Ok(handle) => drain_to_outcome(handle).await,
        Err(err) => RpcDispatchOutcome {
            invocation_id: None,
            state: InvocationState::Failed,
            payload_bytes: Vec::new(),
            error: Some(err),
        },
    }
}

/// Trust-domain local dispatch that still binds the inner invocation
/// to an explicit callee/subject pair. `<self>.invoke_remote` uses this
/// when the transport target is a device but the acted-on object is a
/// resource such as a camera, microphone, or display.
pub async fn dispatch_rpc_local_with_subject(
    runtime: &Arc<LocalRuntime>,
    callee_ura: &str,
    subject_ura: &str,
    ability: &str,
    args: Vec<u8>,
) -> RpcDispatchOutcome {
    let caller = AgentIdentity::new(
        "easynet:///r/_system/agent/_system.local",
        UraProfile::EasynetStrictV2,
    );
    let callee = AgentIdentity::new(callee_ura.to_string(), UraProfile::EasynetStrictV2);
    let subject = SubjectIdentity::new(subject_ura.to_string(), UraProfile::EasynetStrictV2);
    let envelope = InvocationEnvelope::from_wire_parts(
        caller,
        callee,
        subject,
        fresh_nonce(),
        CausalContext::None,
        ability,
        &args,
    );
    dispatch_rpc_admitted(
        runtime,
        AdmittedWireDispatch {
            envelope,
            signature: None,
            payload: args,
        },
    )
    .await
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
/// `services::axon_serve::daemon_invocation_service::
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
        fresh_nonce, make_ability, sha256, sign_invocation, signing_key_from_bytes, AgentIdentity,
        AxonError, CausalContext, InvocationLedger, KeyResolver, LedgerSink, LocalRuntime,
        SubjectIdentity, UraProfile,
    };
    use ed25519_dalek::{SigningKey, VerifyingKey};
    use serde_json::Value;

    struct FixedKey(VerifyingKey);
    impl KeyResolver for FixedKey {
        fn resolve(&self, _: &str) -> Result<VerifyingKey, AxonError> {
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
        let env_sdk = InvocationEnvelope::from_wire_parts(
            caller_sdk.clone(),
            callee_sdk.clone(),
            subject_sdk.clone(),
            nonce,
            CausalContext::None,
            ability,
            payload,
        );
        let sig_sdk = sign_invocation(sk, &env_sdk, "test-key");

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
        let _ = env_sdk;
        let _ = sha256(payload); // assertion in test body
        (wire_envelope, ability.to_string(), payload.to_vec())
    }

    #[tokio::test]
    async fn dispatch_rpc_completes_through_axon_runtime_and_persists() {
        let (rt, ledger, sk, _temp) = build_test_runtime();
        rt.register_ability(
            "test.echo",
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await
        .unwrap();

        let payload = serde_json::to_vec(&serde_json::json!({"x": 7})).unwrap();
        let (wire_env, ability, args) = build_wire_envelope(&sk, "test.echo", &payload);
        let dispatch = from_envelope_open(wire_env, ability, args).expect("translate wire");

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
        assert_eq!(records[0].ability_name, "test.echo");
        assert_eq!(records[0].state, "COMPLETED");
        assert_eq!(records[0].caller_ura, "easynet:///r/t/agent/u.alice");
        assert_eq!(records[0].callee_ura, "easynet:///r/t/device/host");
        assert_eq!(records[0].subject_ura, "easynet:///r/t/device/host");
        assert_eq!(
            outcome.invocation_id.as_deref(),
            Some(records[0].request_id.as_str())
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
        let dispatch = from_envelope_open(wire_env, ability, args).expect("translate wire");

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
    async fn from_envelope_open_rejects_missing_caller() {
        // Defensive: the daemon should never receive a wire
        // envelope without a caller (admission relies on it), but
        // the shim must reject cleanly instead of panicking when
        // a malformed peer sends one anyway.
        let wire = pb::Envelope {
            caller: None,
            callee: Some(pb::AgentIdentity {
                ura: "easynet:///r/t/device/h".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            caller_signature: Some(pb::CallerSignature {
                algorithm: "ed25519".to_string(),
                signature: vec![0; 64],
                key_id_hint: String::new(),
            }),
            invocation_nonce: vec![0; 16],
            ..Default::default()
        };
        let err = from_envelope_open(wire, "x".to_string(), Vec::new()).unwrap_err();
        assert!(err.to_string().contains("missing caller"));
    }

    #[tokio::test]
    async fn from_envelope_open_rejects_missing_signature() {
        let wire = pb::Envelope {
            caller: Some(pb::AgentIdentity {
                ura: "easynet:///r/t/agent/u.x".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(pb::AgentIdentity {
                ura: "easynet:///r/t/device/h".to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            caller_signature: None,
            invocation_nonce: vec![0; 16],
            ..Default::default()
        };
        let err = from_envelope_open(wire, "x".to_string(), Vec::new()).unwrap_err();
        assert!(err.to_string().contains("missing caller_signature"));
    }

    #[tokio::test]
    async fn dispatch_rpc_local_runs_handler_without_signature() {
        // The daemon-internal self-target path. No envelope, no
        // signature — Axon's SystemAgent binding is used. Ledger
        // still records the row.
        let (rt, ledger, _sk, _temp) = build_test_runtime();
        rt.register_ability(
            "demo.daemon_internal",
            make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        )
        .await
        .unwrap();

        let payload = b"\"hello-from-daemon\"".to_vec();
        let outcome = dispatch_rpc_local(&rt, "demo.daemon_internal", payload.clone()).await;
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
        assert_eq!(records[0].ability_name, "demo.daemon_internal");
        assert_eq!(records[0].state, "COMPLETED");
        assert_eq!(
            outcome.invocation_id.as_deref(),
            Some(records[0].request_id.as_str())
        );
    }

    #[tokio::test]
    async fn dispatch_rpc_local_unknown_ability_returns_in_band_error() {
        let (rt, _ledger, _sk, _temp) = build_test_runtime();
        let outcome = dispatch_rpc_local(&rt, "no.such.thing", b"{}".to_vec()).await;
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
    async fn from_envelope_open_defaults_subject_to_callee_when_missing() {
        // RFC 001 §5.1: subject=None ⇒ subject derives from callee.
        // The boundary helper applies this default so admission
        // never sees an unpopulated subject.
        let (_rt, _ledger, sk, _temp) = build_test_runtime();
        let payload = b"{}".to_vec();
        let (mut wire_env, ability, args) = build_wire_envelope(&sk, "any", &payload);
        wire_env.subject = None;
        let dispatch = from_envelope_open(wire_env, ability, args).expect("translate");
        // Subject URA must equal callee URA per the default.
        assert_eq!(
            dispatch.envelope.subject.ura, dispatch.envelope.callee.ura,
            "subject default must mirror callee per RFC 001 §5.1"
        );
    }
}
