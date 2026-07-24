// EasyNet CLI — invocation_transport — session wire contracts
// ============================================================
//
// File: src/daemon/invocation/bidi/session_wire.rs
// Description: Owns the remaining daemon-session control and streaming
//              frames plus the canonical protobuf invocation carrier.
//              Unary product invocations never use the JSON enum below:
//              they travel as `DispatchCall { request: InvokeRequest }`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use tonic::Status;

use axon_sdk::pb::axon::v1::{invoke_bidi_down::Payload as DownPayload, InvokeBidiDown};

use crate::daemon::invocation::bidi::state::presence::{
    DispatchFrame, PresenceDispatchSession, PresenceRegistry, CANONICAL_SESSION_CARRIER_VERSION,
};

pub(crate) const CANONICAL_CARRIER_REQUIRED_CODE: &str = "CANONICAL_CARRIER_REQUIRED";

/// Resolve one live, canonical reverse-dispatch session from an atomic
/// presence snapshot. Unary, server-stream, and bidi relay all pass this gate.
pub(crate) fn require_canonical_dispatch_session(
    presence: &PresenceRegistry,
    execution_host_ura: &str,
    route_ura: &str,
    surface: &str,
) -> Result<PresenceDispatchSession, Status> {
    let session = presence
        .lookup_dispatch_session(execution_host_ura)
        .ok_or_else(|| {
            Status::failed_precondition(format!(
                "RESOLVE_UNAVAILABLE: {surface} selected execution host `{execution_host_ura}` \
                 for route `{route_ura}` but no live session exists"
            ))
        })?;
    if session.contract_version < CANONICAL_SESSION_CARRIER_VERSION {
        return Err(Status::failed_precondition(format!(
            "{CANONICAL_CARRIER_REQUIRED_CODE}: {surface} selected execution host \
             `{execution_host_ura}` for route `{route_ura}`, but that session negotiated carrier \
             v{}; canonical Invocation relay requires v{CANONICAL_SESSION_CARRIER_VERSION} or newer",
            session.contract_version,
        )));
    }
    Ok(session)
}

/// JSON content descriptor used only by the remaining control and
/// bidirectional-stream frames. Canonical unary dispatch carries Axon's
/// protobuf `ContentEnvelope` directly inside `InvokeRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContentEnvelope {
    pub content_type: String,
    pub encoding: String,
    pub schema_ura: String,
    pub encryption: i32,
    pub key_id: String,
}

impl SessionDispatch {
    /// Single codec for JSON control and streaming session frames.
    /// Product RPC dispatch must use `DispatchCall`, not this JSON codec.
    pub fn encode_frame(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// See [`SessionDispatch::encode_frame`].
    pub fn decode_frame(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

impl SessionContentEnvelope {
    pub fn plaintext_json() -> Self {
        Self {
            content_type: "application/json".to_string(),
            encoding: "identity".to_string(),
            schema_ura: String::new(),
            encryption: 0,
            key_id: String::new(),
        }
    }

    pub fn is_encrypted(&self) -> bool {
        self.encryption != 0
    }
}

/// JSON wire shapes for daemon-owned bootstrap control and bidi input bytes.
///
/// Direction discipline (per PR-N6 spec §"Direction discipline"):
///
///   `Request`          device → hub only — daemon-owned control
///   `RequestResult`    hub → device only — answers a `Request`
///                      with resolved bytes or a typed error
///
/// Canonical dispatch opens and results are protobuf `DispatchCall` and
/// `DispatchResult`; they must never be reintroduced into this JSON control
/// envelope.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionDispatch {
    /// Hub → target device. One incremental input frame for a
    /// previously-opened remote bidi session. `payload` carries raw
    /// bytes; `eof=true` closes the input side after this frame.
    BidiInput {
        call_id: u64,
        payload: Vec<u8>,
        eof: bool,
    },
    /// Device → hub daemon-owned control request. Product invocations use
    /// protobuf `ReverseDispatchCall` instead.
    ///
    /// `call_id` is a 16-byte OsRng nonce; concurrent in-flight
    /// Requests are matched on `call_id` against an
    /// `oneshot::Receiver` table. No fairness scheduling — devices
    /// typically have ≤1 concurrent CLI invoke in flight.
    Request {
        call_id: [u8; 16],
        ability_ura: String,
        args: Vec<u8>,
        args_content_envelope: SessionContentEnvelope,
    },
    /// Hub → device. Reverse direction of `Request`. The hub
    /// resolved the target via its PresenceRegistry (same-realm
    /// fast-path) or via cross-hub dial (target realm differs)
    /// and is returning the result bytes — or a typed error
    /// describing why resolution failed.
    RequestResult {
        call_id: [u8; 16],
        outcome: RequestOutcome,
    },
}

/// Outcome of a reverse session request resolved on the hub side. Boundary type
/// over a primitive `(Vec<u8>, Option<String>)` tuple so the
/// discriminator is structural — a malformed wire frame can't
/// produce an ambiguous "empty bytes plus empty error" state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RequestOutcome {
    /// Hub resolved the target and returned bytes.
    Ok { result_bytes: Vec<u8> },
    /// Hub failed to resolve. The error is a typed enum so a
    /// device-side script can distinguish the four common modes
    /// without parsing free-form strings.
    Err { error: SessionRequestError },
}

/// Why a reverse session request failed on the hub side.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionRequestError {
    /// Hub's presence registry has no entry for the target URA.
    TargetOffline,
    /// Hub-side admission rejected the call.
    PermissionDenied { reason: String },
    /// Hub's cross-hub dial failed (peer hub down, TLS handshake
    /// failure, etc.).
    UpstreamFailure { reason: String },
    /// Hub timeout waiting for resolved bytes from upstream.
    UpstreamTimeout,
}

/// Render a 16-byte `Request` / `RequestResult` `call_id` as a
/// 32-character lowercase hex string for op-event output. Stamped
/// into `kind = session_accept_request_frame` and the
/// reverse-session event so operators can correlate hub-side dispatch
/// with the device-side stream. Hex round-trips without escaping.
#[must_use]
pub fn call_id_hex(call_id: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in call_id {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Build a canonical session dispatch. The complete `InvokeRequest`
/// rides the wire unchanged; no inner JSON envelope is created.
pub(crate) fn build_carrier_v1_dispatch_frame(
    call_id: u64,
    request: axon_sdk::pb::axon::v1::InvokeRequest,
    open_bidi: bool,
) -> DispatchFrame {
    use axon_sdk::pb::axon::v1::DispatchCall;
    DispatchFrame::normal(InvokeBidiDown {
        payload: Some(DownPayload::DispatchCall(DispatchCall {
            call_id,
            request: Some(request),
            open_bidi,
        })),
        ..InvokeBidiDown::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_sdk::pb::axon::v1::{
        invoke_bidi_down::Payload, AgentIdentity, CallerSignature, Envelope, InvokeRequest,
    };
    use prost::Message as _;

    #[test]
    fn canonical_carrier_preserves_complete_request() {
        let descriptor_ref = format!(
            "easynet:///r/realm/ability/device.callee.echo@1.0.0#{}!invoke",
            "aa".repeat(32)
        );
        let request = InvokeRequest {
            envelope: Some(Envelope {
                caller: Some(AgentIdentity {
                    ura: "easynet:///r/realm/device/caller".into(),
                    profile: "axon-strict-v2".into(),
                }),
                callee: Some(AgentIdentity {
                    ura: "easynet:///r/realm/device/callee".into(),
                    profile: "axon-strict-v2".into(),
                }),
                invocation_nonce: vec![7; 16],
                caller_signature: Some(CallerSignature {
                    algorithm: "ed25519".into(),
                    signature: vec![9; 64],
                    key_id_hint: "caller".into(),
                }),
                ..Envelope::default()
            }),
            target: Some(
                crate::daemon::invocation::dispatch::invocation_wire::wire_invocation_target(
                    descriptor_ref,
                    "echo",
                )
                .expect("typed descriptor target"),
            ),
            arguments: b"opaque".to_vec(),
            timeout_seconds: 17,
            ..InvokeRequest::default()
        };
        let expected_wire = request.encode_to_vec();
        let frame = build_carrier_v1_dispatch_frame(42, request, false);
        let Some(Payload::DispatchCall(call)) = frame.frame.payload else {
            panic!("expected DispatchCall");
        };
        assert_eq!(call.call_id, 42);
        let relayed = call.request.expect("relayed request");
        assert_eq!(relayed.encode_to_vec(), expected_wire);
    }

    #[test]
    fn presence_registry_rejects_retired_carrier_versions() {
        use crate::daemon::invocation::bidi::state::presence::SessionContract;

        let presence = PresenceRegistry::new();
        let host = "easynet:///r/realm/device/target";
        let (v0_sender, _v0_receiver) = tokio::sync::mpsc::channel(1);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            presence
                .insert_negotiated(
                    host.to_string(),
                    v0_sender,
                    SessionContract {
                        version: 0,
                        claimant_boot_nonce: vec![1; 16],
                    },
                )
                .expect("retired carrier must panic before registration");
        }));
        assert!(panic.is_err(), "v0 sessions must never enter live presence");
    }

    #[test]
    fn canonical_dispatch_session_gate_accepts_every_carrier() {
        use crate::daemon::invocation::bidi::state::presence::SessionContract;

        let presence = PresenceRegistry::new();
        let host = "easynet:///r/realm/device/target";

        let (v1_sender, _v1_receiver) = tokio::sync::mpsc::channel(1);
        let registration = presence
            .insert_negotiated(
                host.to_string(),
                v1_sender,
                SessionContract {
                    version: CANONICAL_SESSION_CARRIER_VERSION,
                    claimant_boot_nonce: vec![2; 16],
                },
            )
            .expect("canonical presence key");
        for surface in ["Invoke", "InvokeStream", "InvokeBidi"] {
            let session =
                require_canonical_dispatch_session(&presence, host, "route-ref::test", surface)
                    .expect("v1 session is canonical for every carrier");
            assert_eq!(session.session_id, registration.session_id);
            assert_eq!(session.contract_version, CANONICAL_SESSION_CARRIER_VERSION);
        }
    }
}
