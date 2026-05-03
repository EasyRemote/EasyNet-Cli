// EasyNet CLI — axon_serve — <self>.invoke_remote initiator (device side)
// =========================================================================
//
// File: src/services/axon_serve/invoke_remote_initiator.rs
// Description: Device-side caller for `<self>.invoke_remote`. Opens a
//              per-call `InvokeBidi` stream against the daemon, sends
//              frame 0 = `EnvelopeOpen` carrying the cross-device
//              dispatch request, drains result frames into a returned
//              `Stream<Item = Bytes>`.
//
// Where this fits in RFC-003
// --------------------------
// PR-1 lands the daemon-side dispatcher. PR-3 (this
// commit) lands two halves of `<self>.invoke_remote`:
//
//   commit 2/3 (this file) — device-side initiator: a function any
//   in-process consumer can call to invoke an ability on a remote
//   device through the local daemon, without knowing the gRPC plumbing
//
//   commit 3/3 (next)      — hub-side handler: the `<self>.invoke_remote`
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
//   target.ability_name = "<self>.invoke_remote"
//   initial_args        = JSON-encoded:
//     {
//       "type": "request",
//       "subject_device": "<canonical device URI>",
//       "ability":        "<ability the remote device runs>",
//       "args":           <bytes — opaque to invoke_remote handler>
//     }
//   streams = [{stream_id: 0, content_type: "application/json", ordering: STRICT}]
//
// Frame 0 down (BinaryChunk on stream 0): JSON-encoded
//   {
//     "type":     "result" | "chunk",
//     "payload":  <bytes>,
//     "terminal": <bool>,    // present on "result" only
//     "error":    <string?>  // present on "result" only
//   }
//
// The MVP-style framing is preserved verbatim (per PR-3 sub-spec §2.3
// and letter 16 — invoke_remote keeps MVP-shape, federation.forward_invoke
// keeps its base64 wrapping).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::pin::Pin;

use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic::Status;

use crate::pb::axon::v1::invocation_client::InvocationClient;
#[cfg(test)]
use crate::pb::axon::v1::BinaryChunk;
use crate::pb::axon::v1::{
    invoke_bidi_up::Payload as UpPayload, EnvelopeOpen, InvocationTarget, InvokeBidiUp,
    StreamDescriptor,
};

/// Daemon-side ability name this initiator targets. The daemon's
/// `InvokeBidi` dispatcher routes on `EnvelopeOpen.target.ability_name`
/// and the `<self>.invoke_remote` arm is the hub-side handler PR-3
/// commit 3/3 lands.
pub const ABILITY_INVOKE_REMOTE: &str = "<self>.invoke_remote";

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

/// Frame-0 payload shape — what JSON gets serialised into
/// `EnvelopeOpen.initial_args`. Public so PR-3 commit 3/3's hub-side
/// handler imports the same type for deserialisation, guaranteeing
/// device-side and hub-side parse the same shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InvokeRemoteUp {
    /// The only frame-0 variant — start a cross-device dispatch.
    Request {
        /// Canonical URI of the device whose `<self>.session` stream
        /// the daemon must look up via `PresenceRegistry::lookup`.
        subject_device: String,
        /// Ability name the remote device should run.
        ability: String,
        /// Opaque payload bytes the remote ability consumes. The
        /// invoke_remote initiator and handler do not interpret these.
        args: Vec<u8>,
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
    /// in transit.
    Result {
        payload: Vec<u8>,
        error: Option<String>,
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

/// Wire shape of a frame the hub's `<self>.invoke_remote` handler
/// pushes down a target device's `<self>.session` reverse channel,
/// and of the matching reply the target device sends back up its
/// session stream.
///
/// MVP-style framing per PR-3 sub-spec §2.3. Public so the
/// `<self>.session` accept handler imports the same type to
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
    /// `<self>.invoke_remote` caller."
    Dispatch {
        call_id: u64,
        ability: String,
        args: Vec<u8>,
    },
    /// Hub → target device. Open one long-lived local bidi handler
    /// on the target and bind it to `call_id`. Used by the
    /// same-hub `fleet.file_transfer` bridge: the hub forwards the
    /// backend's InvokeBidi open to the device's local
    /// `fleet.file_transfer` ability, then streams caller input via
    /// `BidiInput` and target output back via non-terminal
    /// `Result` frames.
    BidiOpen {
        call_id: u64,
        ability: String,
        args: Vec<u8>,
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
    },
    /// Device → hub. A device-mode daemon emits this when its
    /// CLI's `ability invoke --node` lands a `forward_invoke`
    /// that the device's local PresenceRegistry cannot serve
    /// (which is always the case for device-mode, since
    /// device-mode only dials outbound `<self>.session` and
    /// never accepts inbound bidi). The hub picks the frame up
    /// on the existing `<self>.session` accept handler and runs
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
        ability: String,
        args: Vec<u8>,
    },
    /// Hub → device. Reverse direction of `Request`. The hub
    /// resolved the target via its PresenceRegistry (same-tenant
    /// fast-path) or via cross-hub dial (target tenant differs)
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
    /// Hub's PresenceRegistry has no entry for the target URI
    /// (forwarding to a nonexistent device).
    TargetOffline,
    /// Hub-side `forward_invoke` admission rejected the call
    /// (caller URI not in trust anchor, ability not known, etc.).
    PermissionDenied { reason: String },
    /// Hub's cross-hub dial failed (peer hub down, TLS handshake
    /// failure, etc.).
    UpstreamFailure { reason: String },
    /// Hub timeout waiting for resolved bytes from upstream.
    UpstreamTimeout,
}

/// Render a 16-byte `Request` / `RequestResult` `call_id` as a
/// 32-character lowercase hex string for log-marker output.
/// Used by the locked log lines in PR-N6 spec §"Locked log
/// markers" (`[session-accept] received Request frame call_id=…`,
/// `[axon-serve] forward_invoke escalated up <self>.session bidi:
/// call_id=…`). Operator-facing: hex round-trips through any
/// terminal without escaping.
#[must_use]
pub fn call_id_hex(call_id: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in call_id {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Open an `<self>.invoke_remote` bidi stream against the local
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
    ability: String,
    args: Vec<u8>,
) -> Result<Pin<Box<dyn Stream<Item = Result<InvokeRemoteFrame, Status>> + Send>>, Status> {
    let request = InvokeRemoteUp::Request {
        subject_device,
        ability,
        args,
    };
    let initial_args = serde_json::to_vec(&request)
        .map_err(|err| Status::internal(format!("encode invoke_remote request: {err}")))?;

    let frame0 = build_envelope_open_frame(&initial_args);
    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(UP_CHANNEL_CAPACITY);
    up_tx
        .send(frame0)
        .await
        .map_err(|_| Status::internal("up channel closed before frame 0 send"))?;

    // Match server-side cap (1 GiB) on both directions. tonic's
    // default 4 MiB caused `OutOfRange: decoded message length too
    // large` mid-stream on cross-hub file transfers; see boot.rs for
    // the rationale on why 1 GiB.
    let mut client = InvocationClient::new(channel)
        .max_decoding_message_size(
            crate::services::axon_serve::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        )
        .max_encoding_message_size(
            crate::services::axon_serve::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
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
/// `<self>.invoke_remote` ability. AXIOM admission fields stay empty
/// at this layer; the daemon's `AdmissionFacade::verify_envelope`
/// (PR-1 commit 7b/9) gates membership, and PR-7 will add the
/// signature path that fills `mac` in.
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
    S: Stream<Item = Result<crate::pb::axon::v1::InvokeBidiDown, Status>> + Send + Unpin + 'static,
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

fn map_one_frame(frame: &crate::pb::axon::v1::InvokeBidiDown) -> FrameOutcome {
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
        } => FrameOutcome::Terminal(Err(Status::aborted(format!(
            "invoke_remote remote error: {msg}"
        )))),
        InvokeRemoteDown::Result {
            payload,
            error: None,
        } => FrameOutcome::Terminal(Ok(InvokeRemoteFrame::Done(payload))),
    }
}

/// Extract `BinaryChunk.data` bytes from a down frame, returning
/// `None` for non-chunk frames (Receipt / Control) the consumer
/// doesn't see.
fn extract_chunk_bytes(frame: &crate::pb::axon::v1::InvokeBidiDown) -> Option<&[u8]> {
    use crate::pb::axon::v1::invoke_bidi_down::Payload;
    match frame.payload.as_ref()? {
        Payload::BinaryChunk(chunk) => Some(chunk.data.as_slice()),
        Payload::Receipt(_) | Payload::Control(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pb::axon::v1::{invoke_bidi_down::Payload as DownPayload, InvokeBidiDown};
    use futures::stream;

    #[test]
    fn frame_zero_carries_ability_name_and_one_stream_descriptor() {
        let request_json = serde_json::to_vec(&InvokeRemoteUp::Request {
            subject_device: "easynet:///r/realm/agent/dev-B".into(),
            ability: "echo".into(),
            args: b"hi".to_vec(),
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
        assert_eq!(envelope_open.initial_args, request_json);
    }

    #[test]
    fn invoke_remote_up_request_serde_round_trip() {
        let original = InvokeRemoteUp::Request {
            subject_device: "easynet:///r/realm/agent/dev-X".into(),
            ability: "fs.read".into(),
            args: vec![1, 2, 3, 255],
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
        };
        let bytes = serde_json::to_vec(&result_ok).unwrap();
        let recovered: InvokeRemoteDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result_ok, recovered);

        let result_err = InvokeRemoteDown::Result {
            payload: Vec::new(),
            error: Some("target offline".into()),
        };
        let bytes = serde_json::to_vec(&result_err).unwrap();
        let recovered: InvokeRemoteDown = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result_err, recovered);
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
        use crate::pb::axon::v1::InvocationReceipt;
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
                },
            )),
        ];
        let down = stream::iter(frames);
        let (up_tx, _up_rx) = mpsc::channel(1);
        let mut mapped = Box::pin(map_down_stream(down, up_tx));

        let first = mapped.next().await.expect("one frame");
        let status = first.expect_err("must be Err for error result");
        assert_eq!(status.code(), tonic::Code::Aborted);
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
        use crate::pb::axon::v1::{invoke_bidi_down::Payload as DownPayload, BidiControl};
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
            ability: "fs.read".into(),
            args: br#"{"path":"/etc/hosts"}"#.to_vec(),
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
                reason: "caller URI not in trust anchor".into(),
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
        // `(call_id, ability, args)` shape but flow on
        // opposite directions. The wire-level discriminator
        // is the `type` tag; an existing peer that never
        // saw the new variants will see `{"type":"request",
        // ...}` and reject it as unknown rather than
        // misinterpreting it as a `Dispatch` frame. This test
        // pins the tag value so a rename in
        // `#[serde(rename_all = "snake_case")]` shows up here.
        let req = SessionDispatch::Request {
            call_id: [0; 16],
            ability: "x".into(),
            args: vec![],
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
