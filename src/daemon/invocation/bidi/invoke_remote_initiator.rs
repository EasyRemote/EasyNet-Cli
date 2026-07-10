// EasyNet CLI — invocation_transport — runtime.invoke_remote initiator (device side)
// =========================================================================
//
// File: src/daemon/invocation/invoke_remote_initiator.rs
// Description: Device-side caller for `runtime.invoke_remote`. Opens a
//              per-call `InvokeBidi` stream against the daemon, sends
//              frame 0 = `EnvelopeOpen` carrying the cross-device
//              dispatch request, drains result frames into a returned
//              `Stream<Item = Bytes>`.
//
// Where this fits in RFC-003
// --------------------------
// PR-1 lands the daemon-side dispatcher. PR-3 (this
// commit) lands two halves of `runtime.invoke_remote`:
//
//   commit 2/3 (this file) — device-side initiator: a function any
//   in-process consumer can call to invoke an ability on a remote
//   device through the local daemon, without knowing the gRPC plumbing
//
//   commit 3/3 (next)      — hub-side handler: the `runtime.invoke_remote`
//   arm of the daemon's `InvokeBidi` dispatcher that consumes the
//   stream this initiator opens
//
// PR-3 commit 1/3 (the integration test that drives both halves) lands
// after commit 9/9 of PR-1 because it spawns a real daemon binary; the
// initiator + handler are independently testable beforehand with mock
// gRPC channels.
//
// Wire shape (from PR-3 sub-spec §2.1)
// ------------------------------------
// Frame 0 up (`EnvelopeOpen`):
//   target.ability_name = "runtime.invoke_remote"
//   initial_args        = JSON-encoded:
//     {
//       "type": "request",
//       "subject_device": "<canonical device URA>",
//       "ability_ura":    "<canonical Ability URA the remote owner runs>",
//       "args":           <bytes — opaque to invoke_remote handler>
//     }
//   streams = [{stream_id: 0, content_type: "application/json", ordering: STRICT}]
//
// Frame 0 down (BinaryChunk on stream 0): JSON-encoded
//   {
//     "type":     "result" | "chunk",
//     "payload":  <bytes>,
//     "terminal": <bool>,    // present on "result" only
//     "error":    <string?>, // human-readable terminal reason
//     "failure":  <SessionFailure?> // typed canonical projection
//   }
//
// The MVP-style framing is preserved verbatim (per PR-3 sub-spec §2.3
// and letter 16 — invoke_remote keeps MVP-shape, federation.forward_invoke
// keeps its base64 wrapping).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::{collections::HashMap, pin::Pin};

use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic::{Response, Status};

use crate::daemon::invocation::bidi::state::session_failure::SessionFailure;

use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, BinaryChunk, InvokeBidiDown,
};

use crate::daemon::invocation::admission::peer_envelope_signer::decode_inner_envelope;
use crate::daemon::invocation::bidi::state::presence::DispatchFrame;
use crate::daemon::invocation::dispatch::invocation_wire::BoxedDownStream;
use easynet_axon::pb::axon::v1::{
    invoke_bidi_up::Payload as UpPayload, ContentEnvelope, EnvelopeOpen, InvocationTarget,
    InvokeBidiUp, StreamDescriptor,
};

/// Daemon-side ability name this initiator targets. The daemon's
/// `InvokeBidi` dispatcher routes on
/// `EnvelopeOpen.target.ability_name`.
///
/// `runtime.invoke_remote` is the daemon-owned per-call remote
/// dispatch carrier. Backend and CLI track this string verbatim; no
/// historical caller-relative alias is accepted.
pub const ABILITY_INVOKE_REMOTE: &str =
    crate::daemon::ability::names::federation::RUNTIME_INVOKE_REMOTE;

/// Stream id used by every BinaryChunk on the invoke_remote bidi.
/// PR-3 sub-spec §2.1 declares one StreamDescriptor (id=0,
/// `application/json`); every subsequent chunk uses 0.
pub const INVOKE_REMOTE_STREAM_ID: u32 = 0;

const REASON_BIDI_DOWN_SEQUENCE: &str = "AXON_BIDI_DOWN_SEQUENCE";

/// Channel capacity for the up-direction `InvokeBidiUp` mpsc that
/// feeds the gRPC client. Frame 0 plus a small buffer of follow-on
/// chunks if the caller streams arguments — most invoke_remote calls
/// fit in 1 frame.
const UP_CHANNEL_CAPACITY: usize = 8;

/// JSON-serializable content contract for inner invoke_remote
/// arguments. We intentionally do not embed the prost-generated
/// `ContentEnvelope` here because SessionDispatch is serde JSON,
/// while prost messages are the gRPC frame contract. The field
/// names mirror axon.v1.ContentEnvelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContentEnvelope {
    pub content_type: String,
    pub encoding: String,
    pub schema_ura: String,
    pub encryption: i32,
    pub key_id: String,
}

impl SessionDispatch {
    /// Single wire codec for hub<->device session frames.
    ///
    /// Every production frame crosses this fence — to-be-fix.spec §A2
    /// (T2.1) swaps the carrier HERE, in one place, when the JSON
    /// envelope retires for the canonical proto shape. Do not call
    /// serde_json on a SessionDispatch anywhere else.
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

/// Frame-0 payload shape — what JSON gets serialised into
/// `EnvelopeOpen.initial_args`. Public so PR-3 commit 3/3's hub-side
/// handler imports the same type for deserialisation, guaranteeing
/// device-side and hub-side parse the same shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InvokeRemoteUp {
    /// The only frame-0 variant — start a cross-device dispatch.
    Request {
        /// Canonical URA of the device whose `session.open` stream
        /// the daemon must look up via `PresenceRegistry::lookup`.
        subject_device: String,
        /// Inner ability subject. Resource-backed abilities use this
        /// as their envelope subject after the target daemon dispatches
        /// through its LocalRuntime.
        subject_ura: String,
        /// Canonical Ability URA the remote owner should run.
        ability_ura: String,
        /// Opaque payload bytes the remote ability consumes. The
        /// invoke_remote initiator and handler do not interpret these.
        args: Vec<u8>,
        /// Content contract for `args`.
        args_content_envelope: SessionContentEnvelope,
        /// Inner invocation metadata. Carries authority material such
        /// as `x-easynet-delegation` when the inner subject is a user
        /// represented by a hub/backend caller.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        metadata: HashMap<String, String>,
        /// Typed browser-signed user identity (DEC-EU user-caller
        /// pass-through). This first-class field is the only authority
        /// carrier for origin-caller dispatch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin_caller:
            Option<crate::daemon::invocation::admission::origin_caller::OriginCallerClaim>,
    },
}

/// Down-frame payload shape — what JSON the daemon's hub-side handler
/// emits in `BinaryChunk.data` frames on the down stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InvokeRemoteDown {
    /// Intermediate streaming frame from a streaming target ability
    /// (e.g. PTY output, voice frames). Initiator yields the payload
    /// to its consumer; non-terminal.
    Chunk { payload: Vec<u8> },
    /// Terminal frame. `payload` carries the final reply if any;
    /// `error` non-None means the call failed at the remote side or
    /// in transit. `request_id` is the target Axon runtime ledger key
    /// when the target device routed the call through LocalRuntime.
    Result {
        payload: Vec<u8>,
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<SessionFailure>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
}

/// One result frame yielded to the initiator's consumer. Distinct
/// type so consumers don't have to match on a serde-tagged enum at
/// every yield site — the consumer just sees a sequence of `Chunk`
/// frames terminated by either an `Ok` or an `Err`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeRemoteFrame {
    /// Intermediate streaming frame; non-terminal.
    Chunk(Vec<u8>),
    /// Terminal frame with the remote's final reply payload.
    Done(Vec<u8>),
}

/// Wire shape of a frame the hub's `runtime.invoke_remote` handler
/// pushes down a target device's `session.open` reverse channel,
/// and of the matching reply the target device sends back up its
/// session stream.
///
/// MVP-style framing per PR-3 sub-spec §2.3. Public so the
/// `session.open` accept handler imports the same type to
/// recognise these frames in the session stream.
///
/// Direction discipline (per PR-N6 spec §"Direction discipline"):
///
///   `Dispatch`         hub → device only
///   `Result`           device → hub only
///   `Request`          device → hub only — escalates a
///                      `forward_invoke` from a device-mode
///                      daemon up to its hub
///   `RequestResult`    hub → device only — answers a `Request`
///                      with resolved bytes or a typed error
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionDispatch {
    /// Hub → target device. "Run this ability locally and reply
    /// with `call_id` so I can route the reply back to the
    /// `runtime.invoke_remote` caller."
    Dispatch {
        call_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        callee_ura: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject_ura: Option<String>,
        ability: String,
        args: Vec<u8>,
        args_content_envelope: SessionContentEnvelope,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        metadata: HashMap<String, String>,
        /// Typed browser-signed user identity, forwarded verbatim from
        /// `InvokeRemoteUp::Request.origin_caller`. The target device
        /// verifies it and runs the ability with the real user as
        /// Caller.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin_caller:
            Option<crate::daemon::invocation::admission::origin_caller::OriginCallerClaim>,
    },
    /// Hub → target device. Open one long-lived local bidi handler
    /// on the target and bind it to `call_id`. Used by the
    /// same-hub `fs.transfer` bridge: the hub forwards the
    /// backend's InvokeBidi open to the device's local
    /// `fs.transfer` ability, then streams caller input via
    /// `BidiInput` and target output back via non-terminal
    /// `Result` frames.
    BidiOpen {
        call_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        callee_ura: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject_ura: Option<String>,
        ability: String,
        args: Vec<u8>,
        args_content_envelope: SessionContentEnvelope,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        metadata: HashMap<String, String>,
    },
    /// Hub → target device. One incremental input frame for a
    /// previously-opened remote bidi session. `payload` carries raw
    /// bytes; `eof=true` closes the input side after this frame.
    BidiInput {
        call_id: u64,
        payload: Vec<u8>,
        eof: bool,
    },
    /// Target device → hub. The target ran the ability and is
    /// returning the reply. The session task sees this on the
    /// session up stream and routes it via
    /// `PendingDispatchMap::complete(call_id, …)`.
    Result {
        call_id: u64,
        payload: Vec<u8>,
        terminal: bool,
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<SessionFailure>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    /// Device → hub. A device-mode daemon emits this when its
    /// CLI's `ability invoke --node` lands a `forward_invoke`
    /// that the device's local PresenceRegistry cannot serve
    /// (which is always the case for device-mode, since
    /// device-mode only dials outbound `session.open` and
    /// never accepts inbound bidi). The hub picks the frame up
    /// on the existing `session.open` accept handler and runs
    /// the same `forward_invoke` logic against the hub's
    /// authoritative PresenceRegistry, then answers with a
    /// matching `RequestResult` frame.
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

/// Outcome of a `SessionDispatch::Request` resolved on the hub
/// side. Matches PR-N6 spec §"Wire shape" exactly. Boundary type
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

/// Why a `SessionDispatch::Request` failed on the hub side.
/// Mirrors PR-N6 spec §"Wire shape" enum verbatim.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionRequestError {
    /// Hub's PresenceRegistry has no entry for the target URA
    /// (forwarding to a nonexistent device).
    TargetOffline,
    /// Hub-side `forward_invoke` admission rejected the call
    /// (caller URA not in trust anchor, ability not known, etc.).
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
/// `forward_invoke_escalated_up_session_bidi` event so SRE can
/// correlate hub-side dispatch with the device-side bidi stream
/// the call rode out on. Operator-facing: hex round-trips through
/// any terminal without escaping.
#[must_use]
pub fn call_id_hex(call_id: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in call_id {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Open an `runtime.invoke_remote` bidi stream against the local
/// daemon on `channel` and return a stream of result frames.
///
/// The returned stream yields `Ok(InvokeRemoteFrame::Chunk(_))` for
/// each non-terminal chunk and exactly one terminal frame:
/// `Ok(InvokeRemoteFrame::Done(_))` on success or `Err(Status)` on
/// any of:
///
///   * gRPC transport failure (channel down, peer reset)
///   * Frame deserialise failure (malformed JSON in BinaryChunk)
///   * Remote handler reported an error (`InvokeRemoteDown::Result`
///     with `error: Some(_)` — surfaced as `Status::aborted`)
///
/// After the terminal frame the stream is closed; consumers should
/// stop polling.
pub async fn invoke_remote(
    channel: Channel,
    subject_device: String,
    ability_ura: String,
    args: Vec<u8>,
) -> Result<Pin<Box<dyn Stream<Item = Result<InvokeRemoteFrame, Status>> + Send>>, Status> {
    let request = InvokeRemoteUp::Request {
        subject_ura: subject_device.clone(),
        subject_device,
        ability_ura,
        args,
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: HashMap::new(),
        origin_caller: None,
    };
    let initial_args = serde_json::to_vec(&request)
        .map_err(|err| Status::internal(format!("encode invoke_remote request: {err}")))?;

    let frame0 = build_envelope_open_frame(&initial_args);
    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(UP_CHANNEL_CAPACITY);
    up_tx
        .send(frame0)
        .await
        .map_err(|_| Status::internal("up channel closed before frame 0 send"))?;

    // Match the server-side transport-envelope cap on both
    // directions. tonic's default 4 MiB caused `OutOfRange: decoded
    // message length too large` mid-stream on cross-hub transfers;
    // boot.rs owns the exact bounded value.
    let mut client = InvocationClient::new(channel)
        .max_decoding_message_size(
            crate::daemon::boot::invocation::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        )
        .max_encoding_message_size(
            crate::daemon::boot::invocation::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        );
    let response = client
        .invoke_bidi(tonic::Request::new(ReceiverStream::new(up_rx)))
        .await?;
    let down = response.into_inner();

    // Hand the up-tx ownership to the returned stream so the bidi
    // stays alive as long as the consumer holds the down stream.
    Ok(Box::pin(map_down_stream(down, up_tx)))
}

/// Build the EnvelopeOpen frame-0 carrying `initial_args` for the
/// `runtime.invoke_remote` ability. This client-side session bootstrap
/// frame does not manufacture Axon runtime admission state. The receiving
/// daemon may run its transport policy gate for wrapper compatibility, but
/// descriptor-bound user calls still enter LocalRuntime through Axon's
/// public signed/external-signed request constructors.
fn build_envelope_open_frame(initial_args: &[u8]) -> InvokeBidiUp {
    let envelope_open = EnvelopeOpen {
        envelope: None,
        target: Some(InvocationTarget {
            ability_name: ABILITY_INVOKE_REMOTE.to_string(),
            ..InvocationTarget::default()
        }),
        initial_args: initial_args.to_vec(),
        args_content_type: "application/json".to_string(),
        streams: vec![StreamDescriptor {
            stream_id: INVOKE_REMOTE_STREAM_ID,
            content_type: "application/json".to_string(),
            codec_params: String::new(),
            ordering: "STRICT".to_string(),
        }],
        metadata: Default::default(),
        // Carrier negotiation (DEC-F004): v0 until T2.1 step 3.
        session_ext: None,
        content_envelope: Some(ContentEnvelope {
            content_type: "application/json".to_string(),
            encoding: "identity".to_string(),
            ..ContentEnvelope::default()
        }),
    };
    InvokeBidiUp {
        sequence: 0,
        mac: Vec::new(),
        payload: Some(UpPayload::EnvelopeOpen(envelope_open)),
    }
}

/// Map the daemon's down stream into `InvokeRemoteFrame`s for the
/// consumer. Holds `up_tx` so the bidi pairing stays alive for the
/// stream's lifetime; dropping the down stream drops the up channel
/// and the daemon observes the close.
///
/// Implementation note: spawned task drives the source stream and
/// pumps results into a bounded mpsc that the consumer drains via
/// `ReceiverStream`. Manual rather than `async_stream` to avoid a
/// new dependency.
fn map_down_stream<S>(
    mut down: S,
    up_tx: mpsc::Sender<InvokeBidiUp>,
) -> impl Stream<Item = Result<InvokeRemoteFrame, Status>> + Send + 'static
where
    S: Stream<Item = Result<easynet_axon::pb::axon::v1::InvokeBidiDown, Status>>
        + Send
        + Unpin
        + 'static,
{
    let (out_tx, out_rx) = mpsc::channel::<Result<InvokeRemoteFrame, Status>>(8);
    tokio::spawn(async move {
        let _keep_up_alive = up_tx;
        let mut expected_sequence = 0u64;
        while let Some(item) = down.next().await {
            let send = match item {
                Err(status) => out_tx.send(Err(status)).await,
                Ok(frame) => {
                    if frame.sequence != expected_sequence {
                        let _ = out_tx
                            .send(Err(Status::failed_precondition(format!(
                                "{REASON_BIDI_DOWN_SEQUENCE}: expected down frame sequence \
                                 {expected_sequence}, got {}",
                                frame.sequence,
                            ))))
                            .await;
                        return;
                    }
                    expected_sequence += 1;
                    match map_one_frame(&frame) {
                        FrameOutcome::Skip => continue,
                        FrameOutcome::Yield(frame_out) => out_tx.send(Ok(frame_out)).await,
                        FrameOutcome::Terminal(result) => {
                            let _ = out_tx.send(result).await;
                            return;
                        }
                    }
                }
            };
            if send.is_err() {
                // Consumer dropped the receiver; nothing left to do.
                return;
            }
        }
        // Source stream ended without a terminal frame — daemon
        // closed the bidi without sending Result. Surface as aborted.
        let _ = out_tx
            .send(Err(Status::aborted(
                "invoke_remote down stream ended before terminal Result frame",
            )))
            .await;
    });
    ReceiverStream::new(out_rx)
}

/// Per-frame mapping outcome. Splits the decision (skip / yield /
/// terminate) from the channel-send so the spawned task above stays
/// readable.
enum FrameOutcome {
    /// Frame is non-consumer-visible (Receipt / Control) — drop it
    /// silently and continue draining.
    Skip,
    /// Yield this frame to the consumer; keep draining.
    Yield(InvokeRemoteFrame),
    /// Terminal frame; yield the carried `Result` and stop draining.
    Terminal(Result<InvokeRemoteFrame, Status>),
}

fn map_one_frame(frame: &easynet_axon::pb::axon::v1::InvokeBidiDown) -> FrameOutcome {
    let bytes = match extract_chunk_bytes(frame) {
        Some(b) => b,
        None => return FrameOutcome::Skip,
    };
    let parsed: InvokeRemoteDown = match serde_json::from_slice(bytes) {
        Ok(p) => p,
        Err(err) => {
            return FrameOutcome::Terminal(Err(Status::internal(format!(
                "invoke_remote down-frame decode: {err}"
            ))));
        }
    };
    match parsed {
        InvokeRemoteDown::Chunk { payload } => {
            FrameOutcome::Yield(InvokeRemoteFrame::Chunk(payload))
        }
        InvokeRemoteDown::Result {
            payload: _,
            error: Some(msg),
            failure,
            request_id: _,
        } => {
            let detail = failure
                .map(|failure| failure.status_detail())
                .unwrap_or(msg);
            FrameOutcome::Terminal(Err(Status::aborted(format!(
                "invoke_remote remote error: {detail}"
            ))))
        }
        InvokeRemoteDown::Result {
            payload,
            error: None,
            failure: _,
            request_id: _,
        } => FrameOutcome::Terminal(Ok(InvokeRemoteFrame::Done(payload))),
    }
}

/// Extract `BinaryChunk.data` bytes from a down frame, returning
/// `None` for non-chunk frames (Receipt / Control) the consumer
/// doesn't see.
fn extract_chunk_bytes(frame: &easynet_axon::pb::axon::v1::InvokeBidiDown) -> Option<&[u8]> {
    use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload;
    match frame.payload.as_ref()? {
        Payload::BinaryChunk(chunk) => Some(chunk.data.as_slice()),
        Payload::Receipt(_) | Payload::Control(_) => None,
        // Carrier-v1 frames (DEC-F004): not chunk traffic; dual-read
        // lands in T2.1 steps 2-3.
        Payload::DispatchCall(_) | Payload::ReverseDispatchResult(_) => None,
    }
}

// ── Hub-side frame construction ─────────────────────────────────────
// The dispatch/terminal frame builders and the inner-payload decode
// the hub uses when it drives `runtime.invoke_remote` over a device's
// session channel. Moved next to the wire types they serialize
// (commit-plan-2 E2d).

/// **PR-N1 commit 11/N + C1a**. The inner-envelope payload
/// shape the CLI bridge (`support/federation_invoke.rs::
/// invoke_via_federation_forward`) emits: a JSON object
/// carrying the canonical `(ability_ura, args)` pair the user
/// selected plus a `call_id` minted client-side that DEC-N4
/// §2.1 threads back through `ForwardInvokeResponse.
/// correlation_call_id` so the caller can correlate the
/// response with its awaiting bidi.
pub(crate) struct InnerPayload {
    pub ability_ura: String,
    pub subject_ura: String,
    pub args_bytes: Vec<u8>,
    pub call_id: String,
}

/// **PR-N1 commit 11/N + C1a**. Decode the base64-then-JSON
/// inner payload the CLI bridge ships, surfacing each parse
/// failure as `Status::invalid_argument` with a wire-stable
/// hint so scripts grepping the daemon log can distinguish
/// them. Non-empty `call_id` is required by DEC-N4 §2.1; a
/// missing or empty value rejects with a clear error rather
/// than synthesising a server-side id (which would defeat the
/// caller-side correlation contract).
pub(crate) fn decode_inner_payload(b64: &str) -> Result<InnerPayload, Status> {
    let raw = decode_inner_envelope(b64)?;
    if raw.is_empty() {
        return Err(Status::invalid_argument(
            "federation.forward_invoke: inner_envelope_b64 is empty; \
             cross-hub dispatch requires a base64-encoded JSON \
             {ability_ura, subject_ura, args, call_id} payload",
        ));
    }
    let parsed: serde_json::Value = serde_json::from_slice(&raw).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.forward_invoke: inner envelope is not valid JSON: {err}"
        ))
    })?;
    let obj = parsed.as_object().ok_or_else(|| {
        Status::invalid_argument(
            "federation.forward_invoke: inner envelope must be a JSON object \
             with `ability_ura`, `subject_ura`, `args`, and `call_id` fields",
        )
    })?;
    let ability_ura = obj
        .get("ability_ura")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "federation.forward_invoke: inner envelope is missing a non-empty \
                 string `ability_ura` field",
            )
        })?
        .to_string();
    let call_id = obj
        .get("call_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "federation.forward_invoke: inner envelope is missing a non-empty \
                 string `call_id` field (DEC-N4 §2.1 correlation requirement)",
            )
        })?
        .to_string();
    let subject_ura = obj
        .get("subject_ura")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "federation.forward_invoke: inner envelope is missing a non-empty \
                 string `subject_ura` field",
            )
        })?
        .to_string();
    crate::core::ura::parse_ura(&subject_ura).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.forward_invoke: inner envelope subject_ura is not a canonical URA: {err}"
        ))
    })?;
    let args_value = obj
        .get("args")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let args_bytes = serde_json::to_vec(&args_value).map_err(|err| {
        Status::internal(format!(
            "federation.forward_invoke: re-serialise inner args: {err}"
        ))
    })?;
    Ok(InnerPayload {
        ability_ura,
        subject_ura,
        args_bytes,
        call_id,
    })
}

/// Carrier-v1 dispatch frame (DEC-F004 / T2.1 step 2d): the complete
/// canonical InvokeRequest rides the wire verbatim — no JSON, no
/// unpack/repack, no base64 inflation. The JSON builder below it is
/// the v0 shape and is deleted with the rest of the JSON carrier one
/// release window after step 3.
pub(crate) fn build_carrier_v1_dispatch_frame(
    call_id: u64,
    request: easynet_axon::pb::axon::v1::InvokeRequest,
    open_bidi: bool,
) -> DispatchFrame {
    use easynet_axon::pb::axon::v1::DispatchCall;
    DispatchFrame::normal(InvokeBidiDown {
        payload: Some(DownPayload::DispatchCall(DispatchCall {
            call_id,
            request: Some(request),
            open_bidi,
        })),
        ..InvokeBidiDown::default()
    })
}

/// Build a `DispatchFrame` carrying a `SessionDispatch::Dispatch` JSON
/// payload, ready to push down a target's `session.open` reverse
/// channel. Encoding failure is impossible for the current variant
/// (call_id u64, owned String, owned Vec<u8>) but mapped to
/// `Status::internal` for forward-compatibility per letter 25 §"flag".
pub(crate) struct InvokeRemoteDispatchFrameRequest<'a> {
    pub(crate) call_id: u64,
    pub(crate) callee_ura: &'a str,
    pub(crate) subject_ura: &'a str,
    pub(crate) ability: &'a str,
    pub(crate) args: &'a [u8],
    pub(crate) args_content_envelope: SessionContentEnvelope,
    pub(crate) metadata: HashMap<String, String>,
    pub(crate) origin_caller:
        Option<crate::daemon::invocation::admission::origin_caller::OriginCallerClaim>,
}

pub(crate) fn build_invoke_remote_dispatch_frame(
    request: InvokeRemoteDispatchFrameRequest<'_>,
) -> Result<DispatchFrame, Status> {
    let InvokeRemoteDispatchFrameRequest {
        call_id,
        callee_ura,
        subject_ura,
        ability,
        args,
        args_content_envelope,
        metadata,
        origin_caller,
    } = request;

    let subject_ura = subject_ura.trim();
    if subject_ura.is_empty() {
        return Err(Status::invalid_argument(
            "runtime.invoke_remote: missing inner subject_ura",
        ));
    }

    let payload = SessionDispatch::Dispatch {
        call_id,
        callee_ura: Some(callee_ura.to_string()),
        subject_ura: Some(subject_ura.to_string()),
        ability: ability.to_string(),
        args: args.to_vec(),
        args_content_envelope,
        metadata,
        origin_caller,
    };
    let bytes = payload.encode_frame().map_err(|err| {
        Status::internal(format!(
            "runtime.invoke_remote: encode SessionDispatch::Dispatch: {err}"
        ))
    })?;
    let chunk = BinaryChunk {
        stream_id: INVOKE_REMOTE_STREAM_ID,
        data: bytes,
        ..BinaryChunk::default()
    };
    Ok(DispatchFrame::normal(InvokeBidiDown {
        payload: Some(DownPayload::BinaryChunk(chunk)),
        ..InvokeBidiDown::default()
    }))
}

/// Build the terminal `InvokeBidiDown` frame the
/// `runtime.invoke_remote` caller's down stream yields. Carries the
/// `InvokeRemoteDown::Result` JSON in `BinaryChunk.data`.
pub(crate) fn build_invoke_remote_terminal_frame(
    down: &InvokeRemoteDown,
) -> Result<InvokeBidiDown, Status> {
    let bytes = serde_json::to_vec(down).map_err(|err| {
        Status::internal(format!(
            "runtime.invoke_remote: encode InvokeRemoteDown: {err}"
        ))
    })?;
    let chunk = BinaryChunk {
        stream_id: INVOKE_REMOTE_STREAM_ID,
        data: bytes,
        ..BinaryChunk::default()
    };
    Ok(InvokeBidiDown {
        payload: Some(DownPayload::BinaryChunk(chunk)),
        ..InvokeBidiDown::default()
    })
}

/// Build a one-shot `runtime.invoke_remote` Response stream carrying a
/// single terminal frame whose `InvokeRemoteDown::Result` has
/// `error = Some(msg)` and an empty payload.
///
/// Why this exists: `dispatch_invoke_remote` has two flavours of
/// failure — protocol/structural (malformed frame 0, daemon
/// misconfigured) and operational (target not in registry, target
/// channel full / closed, target handler errored). The protocol /
/// structural ones return a `tonic::Status` (gRPC-level error,
/// surfaces upstream as HTTP 500). The operational ones MUST stay
/// in-band so the caller sees a successful stream that yields a
/// final frame whose `error` field carries the structured reason —
/// otherwise a Go/HTTP shim atop tonic surfaces them as opaque 500s
/// and the human user never sees "target offline", just "500".
/// The post-dispatch failure paths already did this (target session
/// dropped, target replied with error); the pre-dispatch paths used
/// to raise `Status`. This helper aligns both halves under one
/// shape.
pub(crate) fn invoke_remote_inband_error_response(
    msg: String,
) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
    let failure = SessionFailure::from_reason(&msg, "INVOCATION_FAILED", false);
    let down = InvokeRemoteDown::Result {
        payload: Vec::new(),
        error: Some(msg),
        failure: Some(failure),
        request_id: None,
    };
    let frame = build_invoke_remote_terminal_frame(&down)?;
    let (down_tx, down_rx) = mpsc::channel::<Result<InvokeBidiDown, Status>>(1);
    tokio::spawn(async move {
        let _ = down_tx.send(Ok(frame)).await;
    });
    let stream = ReceiverStream::new(down_rx);
    Ok(Response::new(
        Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::pb::axon::v1::{invoke_bidi_down::Payload as DownPayload, InvokeBidiDown};
    use futures::stream;

    #[test]
    fn frame_zero_carries_ability_name_and_one_stream_descriptor() {
        let request_json = serde_json::to_vec(&InvokeRemoteUp::Request {
            subject_device: "easynet:///r/realm/device/dev-B".into(),
            subject_ura: "easynet:///r/realm/device/dev-B".into(),
            ability_ura: "easynet:///r/realm/ability/device.dev-B.echo".into(),
            args: b"hi".to_vec(),
            args_content_envelope: SessionContentEnvelope::plaintext_json(),
            metadata: HashMap::new(),
            origin_caller: None,
        })
        .unwrap();
        let frame = build_envelope_open_frame(&request_json);

        assert_eq!(frame.sequence, 0);
        assert!(frame.mac.is_empty());

        let envelope_open = match frame.payload.as_ref().unwrap() {
            UpPayload::EnvelopeOpen(eo) => eo,
            other => panic!("expected EnvelopeOpen, got {other:?}"),
        };
        assert_eq!(
            envelope_open.target.as_ref().unwrap().ability_name,
            ABILITY_INVOKE_REMOTE,
        );
        assert_eq!(envelope_open.streams.len(), 1);
        assert_eq!(envelope_open.streams[0].stream_id, INVOKE_REMOTE_STREAM_ID);
        assert_eq!(envelope_open.streams[0].content_type, "application/json");
        assert_eq!(envelope_open.args_content_type, "application/json");
        let content = envelope_open
            .content_envelope
            .as_ref()
            .expect("EnvelopeOpen content envelope");
        assert_eq!(content.content_type, "application/json");
        assert_eq!(content.encoding, "identity");
        assert_eq!(envelope_open.initial_args, request_json);
    }

    #[test]
    fn invoke_remote_up_request_serde_round_trip() {
        let original = InvokeRemoteUp::Request {
            subject_device: "easynet:///r/realm/device/dev-X".into(),
            subject_ura: "easynet:///r/realm/resource/camera-1".into(),
            ability_ura: "easynet:///r/realm/ability/device.dev-X.fs.read".into(),
            args: vec![1, 2, 3, 255],
            args_content_envelope: SessionContentEnvelope::plaintext_json(),
            metadata: {
                let mut metadata = HashMap::new();
                metadata.insert("x-easynet-delegation".to_string(), "proof".to_string());
                metadata
            },
            origin_caller: None,
        };
        let bytes = serde_json::to_vec(&original).unwrap();
        let recovered: InvokeRemoteUp = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(original, recovered);
    }

    #[test]
    fn invoke_remote_down_chunk_and_result_round_trip() {
        let chunk = InvokeRemoteDown::Chunk {
            payload: b"streaming-output".to_vec(),
        };
        let bytes = serde_json::to_vec(&chunk).unwrap();
        let recovered: InvokeRemoteDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(chunk, recovered);

        let result_ok = InvokeRemoteDown::Result {
            payload: b"final-reply".to_vec(),
            error: None,
            failure: None,
            request_id: None,
        };
        let bytes = serde_json::to_vec(&result_ok).unwrap();
        let recovered: InvokeRemoteDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result_ok, recovered);

        let result_err = InvokeRemoteDown::Result {
            payload: Vec::new(),
            error: Some("target offline".into()),
            failure: Some(SessionFailure::from_reason(
                "target offline",
                "TARGET_NOT_IN_PRESENCE_REGISTRY",
                true,
            )),
            request_id: None,
        };
        let bytes = serde_json::to_vec(&result_err).unwrap();
        let recovered: InvokeRemoteDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result_err, recovered);
    }

    #[test]
    fn session_failure_status_detail_keeps_code_without_duplicate_parsing() {
        let failure = SessionFailure::from_explicit("disk_full", "volume is full", true);
        assert_eq!(failure.status_detail(), "DISK_FULL: volume is full");

        let empty_message = SessionFailure::from_explicit("device_removed", "", false);
        assert_eq!(empty_message.status_detail(), "DEVICE_REMOVED");
    }

    /// Build a synthetic down frame carrying `payload` as the
    /// `InvokeRemoteDown` JSON in `BinaryChunk.data`. Tests use this
    /// to drive the down-stream mapper without a real gRPC server.
    fn down_chunk_with(sequence: u64, payload: InvokeRemoteDown) -> InvokeBidiDown {
        let json = serde_json::to_vec(&payload).expect("encode test payload");
        InvokeBidiDown {
            sequence,
            mac: Vec::new(),
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                stream_id: INVOKE_REMOTE_STREAM_ID,
                data: json,
                pts: 0,
            })),
        }
    }

    fn down_receipt(sequence: u64) -> InvokeBidiDown {
        use easynet_axon::pb::axon::v1::InvocationReceipt;
        InvokeBidiDown {
            sequence,
            mac: Vec::new(),
            payload: Some(DownPayload::Receipt(InvocationReceipt::default())),
        }
    }

    #[tokio::test]
    async fn map_down_stream_yields_done_for_clean_terminal_result() {
        let frames = vec![
            Ok(down_receipt(0)),
            Ok(down_chunk_with(
                1,
                InvokeRemoteDown::Result {
                    payload: b"the-reply".to_vec(),
                    error: None,
                    failure: None,
                    request_id: None,
                },
            )),
        ];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("one frame");
        assert_eq!(
            first.unwrap(),
            InvokeRemoteFrame::Done(b"the-reply".to_vec())
        );
        assert!(
            mapped.next().await.is_none(),
            "stream should end after terminal"
        );
    }

    #[tokio::test]
    async fn map_down_stream_yields_chunks_then_done() {
        let frames = vec![
            Ok(down_receipt(0)),
            Ok(down_chunk_with(
                1,
                InvokeRemoteDown::Chunk {
                    payload: b"first".to_vec(),
                },
            )),
            Ok(down_chunk_with(
                2,
                InvokeRemoteDown::Chunk {
                    payload: b"second".to_vec(),
                },
            )),
            Ok(down_chunk_with(
                3,
                InvokeRemoteDown::Result {
                    payload: b"final".to_vec(),
                    error: None,
                    failure: None,
                    request_id: None,
                },
            )),
        ];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        assert_eq!(
            mapped.next().await.unwrap().unwrap(),
            InvokeRemoteFrame::Chunk(b"first".to_vec()),
        );
        assert_eq!(
            mapped.next().await.unwrap().unwrap(),
            InvokeRemoteFrame::Chunk(b"second".to_vec()),
        );
        assert_eq!(
            mapped.next().await.unwrap().unwrap(),
            InvokeRemoteFrame::Done(b"final".to_vec()),
        );
        assert!(mapped.next().await.is_none());
    }

    #[tokio::test]
    async fn map_down_stream_surfaces_remote_error_as_aborted_status() {
        let frames = vec![
            Ok(down_receipt(0)),
            Ok(down_chunk_with(
                1,
                InvokeRemoteDown::Result {
                    payload: Vec::new(),
                    error: Some("device dropped before reply".into()),
                    failure: Some(SessionFailure::from_reason(
                        "device dropped before reply",
                        "DEVICE_REMOVED",
                        true,
                    )),
                    request_id: None,
                },
            )),
        ];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("one frame");
        let status = first.expect_err("must be Err for error result");
        assert_eq!(status.code(), tonic::Code::Aborted);
        assert!(status.message().contains("DEVICE_REMOVED"));
        assert!(status.message().contains("device dropped before reply"));
        assert!(mapped.next().await.is_none());
    }

    #[tokio::test]
    async fn map_down_stream_surfaces_premature_close_as_aborted() {
        // Empty stream — daemon closed without sending a Result.
        let frames: Vec<Result<InvokeBidiDown, Status>> = vec![];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("one frame");
        let status = first.expect_err("premature close must surface as Err");
        assert_eq!(status.code(), tonic::Code::Aborted);
        assert!(status.message().contains("ended before terminal"));
    }

    #[tokio::test]
    async fn map_down_stream_skips_receipt_and_control_frames() {
        use easynet_axon::pb::axon::v1::{invoke_bidi_down::Payload as DownPayload, BidiControl};
        let frames = vec![
            Ok(down_receipt(0)),
            Ok(InvokeBidiDown {
                sequence: 1,
                mac: Vec::new(),
                payload: Some(DownPayload::Control(BidiControl::default())),
            }),
            Ok(down_chunk_with(
                2,
                InvokeRemoteDown::Result {
                    payload: b"reply-after-receipt".to_vec(),
                    error: None,
                    failure: None,
                    request_id: None,
                },
            )),
        ];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("first non-skipped frame");
        assert_eq!(
            first.unwrap(),
            InvokeRemoteFrame::Done(b"reply-after-receipt".to_vec()),
        );
    }

    #[tokio::test]
    async fn map_down_stream_surfaces_malformed_json_as_internal() {
        let bad_frame = InvokeBidiDown {
            sequence: 1,
            mac: Vec::new(),
            payload: Some(DownPayload::BinaryChunk(BinaryChunk {
                stream_id: INVOKE_REMOTE_STREAM_ID,
                data: b"not valid JSON {{{".to_vec(),
                pts: 0,
            })),
        };
        let down = stream::iter(vec![Ok(down_receipt(0)), Ok(bad_frame)]);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("one frame");
        let status = first.expect_err("malformed JSON must surface as Err");
        assert_eq!(status.code(), tonic::Code::Internal);
        assert!(status.message().contains("decode"));
    }

    #[tokio::test]
    async fn map_down_stream_rejects_out_of_sequence_frames() {
        let frames = vec![
            Ok(down_receipt(0)),
            Ok(down_chunk_with(
                2,
                InvokeRemoteDown::Result {
                    payload: b"reply".to_vec(),
                    error: None,
                    failure: None,
                    request_id: None,
                },
            )),
        ];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("sequence error frame");
        let status = first.expect_err("out-of-sequence frame must surface as Err");
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(status.message().contains(REASON_BIDI_DOWN_SEQUENCE));
    }

    #[tokio::test]
    async fn map_down_stream_surfaces_transport_error_verbatim() {
        let frames: Vec<Result<InvokeBidiDown, Status>> =
            vec![Err(Status::unavailable("connection reset"))];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("one frame");
        let status = first.expect_err("transport error must propagate");
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(status.message().contains("connection reset"));
    }

    #[test]
    fn session_dispatch_request_round_trip() {
        // PR-N6 wire shape (C2): Request frame device → hub.
        let original = SessionDispatch::Request {
            call_id: [0xab; 16],
            ability_ura: "easynet:///r/localhost/ability/hub.federation.forward_invoke".into(),
            args: br#"{"resource_ref":{"resource_ura":"easynet:///r/localhost/resource/device.local-device/fs/tmp/hosts","owner_ura":"easynet:///r/localhost/device/local-device","namespace":"fs","display_path":"tmp/hosts","capability":"read","expires_unix_ms":4102444800000,"revision":"fs-local-mapping-v1"}}"#.to_vec(),
            args_content_envelope: SessionContentEnvelope::plaintext_json(),
        };
        let bytes = serde_json::to_vec(&original).expect("encode");
        let recovered: SessionDispatch = serde_json::from_slice(&bytes).expect("decode");
        assert_eq!(original, recovered);
    }

    #[test]
    fn session_dispatch_request_result_ok_round_trip() {
        let original = SessionDispatch::RequestResult {
            call_id: [0x42; 16],
            outcome: RequestOutcome::Ok {
                result_bytes: b"127.0.0.1 localhost\n".to_vec(),
            },
        };
        let bytes = serde_json::to_vec(&original).expect("encode");
        let recovered: SessionDispatch = serde_json::from_slice(&bytes).expect("decode");
        assert_eq!(original, recovered);
    }

    #[test]
    fn session_dispatch_request_result_err_round_trip_each_variant() {
        // Pin every SessionRequestError variant through the
        // serde wire so a future field rename is caught here
        // rather than producing silent decode failures on the
        // device side at runtime.
        let cases = vec![
            SessionRequestError::TargetOffline,
            SessionRequestError::PermissionDenied {
                reason: "caller URA not in trust anchor".into(),
            },
            SessionRequestError::UpstreamFailure {
                reason: "peer hub TLS handshake failed".into(),
            },
            SessionRequestError::UpstreamTimeout,
        ];
        for err in cases {
            let original = SessionDispatch::RequestResult {
                call_id: [0; 16],
                outcome: RequestOutcome::Err { error: err.clone() },
            };
            let bytes = serde_json::to_vec(&original).expect("encode");
            let recovered: SessionDispatch = serde_json::from_slice(&bytes).expect("decode");
            assert_eq!(
                original, recovered,
                "round-trip mismatch for SessionRequestError {err:?}",
            );
        }
    }

    #[test]
    fn session_dispatch_request_carries_distinct_tag_in_serialised_form() {
        // The `Request` and `Dispatch` variants share the
        // `(call_id, ability_ura, args)` shape but flow on
        // opposite directions. The wire-level discriminator
        // is the `type` tag; an existing peer that never
        // saw the new variants will see `{"type":"request",
        // ...}` and reject it as unknown rather than
        // misinterpreting it as a `Dispatch` frame. This test
        // pins the tag value so a rename in
        // `#[serde(rename_all = "snake_case")]` shows up here.
        let req = SessionDispatch::Request {
            call_id: [0; 16],
            ability_ura: "easynet:///r/realm/ability/hub.federation.forward_invoke".into(),
            args: vec![],
            args_content_envelope: SessionContentEnvelope::plaintext_json(),
        };
        let json = serde_json::to_string(&req).expect("encode");
        assert!(
            json.contains(r#""type":"request""#),
            "serialised form must carry type=request tag, got {json}",
        );

        let res = SessionDispatch::RequestResult {
            call_id: [0; 16],
            outcome: RequestOutcome::Ok {
                result_bytes: vec![],
            },
        };
        let json = serde_json::to_string(&res).expect("encode");
        assert!(
            json.contains(r#""type":"request_result""#),
            "serialised form must carry type=request_result tag, got {json}",
        );
    }

    #[test]
    fn call_id_hex_renders_32_lowercase_chars() {
        assert_eq!(call_id_hex(&[0; 16]), "00000000000000000000000000000000");
        assert_eq!(call_id_hex(&[0xff; 16]), "ffffffffffffffffffffffffffffffff");
        // Mixed bytes round-trip through the deterministic
        // 2-char-per-byte format the log markers reference.
        let bytes = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        assert_eq!(call_id_hex(&bytes), "000102030405060708090a0b0c0d0e0f");
    }
}

#[cfg(test)]
mod hub_frame_tests {
    use super::*;

    #[test]
    fn carrier_v1_dispatch_frame_carries_complete_invoke_request() {
        use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest};
        let frame = build_carrier_v1_dispatch_frame(
            42,
            InvokeRequest {
                envelope: Some(Envelope {
                    caller: Some(AgentIdentity {
                        ura: "easynet:///r/t/user/alice".into(),
                        profile: "easynet-strict-v2".into(),
                    }),
                    ..Envelope::default()
                }),
                function_name: "dev.fs.read".into(),
                arguments: b"{\"path\":\"/tmp/x\"}".to_vec(),
                ..InvokeRequest::default()
            },
            false,
        );
        let Some(DownPayload::DispatchCall(call)) = frame.frame.payload else {
            panic!("expected DispatchCall payload");
        };
        assert_eq!(call.call_id, 42);
        assert!(!call.open_bidi);
        let request = call.request.expect("request present");
        assert_eq!(request.function_name, "dev.fs.read");
        assert_eq!(
            request.envelope.unwrap().caller.unwrap().ura,
            "easynet:///r/t/user/alice"
        );
    }

    #[test]
    fn build_invoke_remote_dispatch_frame_carries_session_dispatch_json() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "x-easynet-delegation".to_string(),
            "serialized-proof".to_string(),
        );
        let frame = build_invoke_remote_dispatch_frame(InvokeRemoteDispatchFrameRequest {
            call_id: 42,
            callee_ura: "easynet:///r/realm/device/dev",
            subject_ura: "easynet:///r/realm/resource/camera-1",
            ability: "echo",
            args: b"hello",
            args_content_envelope: SessionContentEnvelope::plaintext_json(),
            metadata,
            origin_caller: None,
        })
        .expect("built");
        let payload = match frame.frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(chunk) => chunk,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(payload.stream_id, INVOKE_REMOTE_STREAM_ID);
        let parsed: SessionDispatch =
            serde_json::from_slice(&payload.data).expect("decode SessionDispatch");
        match parsed {
            SessionDispatch::Dispatch {
                call_id,
                callee_ura,
                subject_ura,
                ability,
                args,
                args_content_envelope,
                metadata,
                origin_caller,
            } => {
                assert_eq!(call_id, 42);
                assert!(origin_caller.is_none(), "no claim attached in this test");
                assert_eq!(callee_ura.as_deref(), Some("easynet:///r/realm/device/dev"));
                assert_eq!(
                    subject_ura.as_deref(),
                    Some("easynet:///r/realm/resource/camera-1")
                );
                assert_eq!(ability, "echo");
                assert_eq!(args, b"hello");
                assert_eq!(args_content_envelope.content_type, "application/json");
                assert_eq!(
                    metadata.get("x-easynet-delegation").map(String::as_str),
                    Some("serialized-proof")
                );
            }
            _ => panic!("expected Dispatch variant"),
        }
    }
    #[test]
    fn build_invoke_remote_terminal_frame_round_trips_done_payload() {
        let down = InvokeRemoteDown::Result {
            payload: b"the-reply".to_vec(),
            error: None,
            failure: None,
            request_id: None,
        };
        let frame = build_invoke_remote_terminal_frame(&down).expect("built");
        let chunk = match frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(c) => c,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(chunk.stream_id, INVOKE_REMOTE_STREAM_ID);
        let parsed: InvokeRemoteDown = serde_json::from_slice(&chunk.data).expect("decode");
        assert_eq!(parsed, down);
    }
    #[test]
    fn build_invoke_remote_terminal_frame_round_trips_chunk_payload() {
        let down = InvokeRemoteDown::Chunk {
            payload: b"screen-frame".to_vec(),
        };
        let frame = build_invoke_remote_terminal_frame(&down).expect("built");
        let chunk = match frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(c) => c,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(chunk.stream_id, INVOKE_REMOTE_STREAM_ID);
        let parsed: InvokeRemoteDown = serde_json::from_slice(&chunk.data).expect("decode");
        assert_eq!(parsed, down);
    }
    #[tokio::test]
    async fn invoke_remote_inband_error_response_surfaces_reason_in_terminal_frame() {
        // Operational failures inside `runtime.invoke_remote` (target
        // offline, channel full, handler errored) used to surface as
        // `tonic::Status` — i.e., a gRPC-level error, which the Go
        // HTTP shim above tonic logs as a bare HTTP 500. The frontend
        // then had nothing to render except "500". The helper used by
        // those sites must instead produce a successful Response
        // carrying ONE InvokeRemoteDown::Result frame whose `error`
        // field carries the structured reason, so the shim sees
        // gRPC success and can serialise the reason to the HTTP body.
        let response = invoke_remote_inband_error_response(
            "target `easynet:///r/test-realm/agent/dev.liangbing` is not in PresenceRegistry"
                .to_string(),
        )
        .expect("helper must return Ok — failure is in-band, not gRPC-level");

        let mut stream = response.into_inner();
        let frame = tokio_stream::StreamExt::next(&mut stream)
            .await
            .expect("stream yields one terminal frame")
            .expect("terminal frame is not a gRPC-level error");

        let chunk = match frame.payload.expect("frame has payload") {
            DownPayload::BinaryChunk(c) => c,
            _ => panic!("expected BinaryChunk"),
        };
        assert_eq!(chunk.stream_id, INVOKE_REMOTE_STREAM_ID);

        let parsed: InvokeRemoteDown =
            serde_json::from_slice(&chunk.data).expect("decode InvokeRemoteDown");
        match parsed {
            InvokeRemoteDown::Result { payload, error, .. } => {
                assert!(payload.is_empty(), "in-band error frame carries no payload");
                let msg = error.expect("error field must be Some(...)");
                assert!(
                    msg.contains("dev.liangbing"),
                    "reason string must round-trip the target URA verbatim — got {msg:?}"
                );
                assert!(
                    msg.contains("not in PresenceRegistry"),
                    "reason string must round-trip the diagnostic verbatim — got {msg:?}"
                );
            }
            other => panic!("expected Result variant, got {other:?}"),
        }

        // Single-frame stream: after the terminal frame, the stream
        // must close (otherwise a caller iterating frames hangs).
        assert!(
            tokio_stream::StreamExt::next(&mut stream).await.is_none(),
            "in-band error stream must be one-shot and close after the terminal frame"
        );
    }
}
