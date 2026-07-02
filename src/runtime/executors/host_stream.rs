// EasyNet CLI — host_stream executor
// =================================================================
//
// File: src/runtime/executors/host_stream.rs
//
// Server-stream executor for the `host_stream` AbilityExec. It lets an
// EXTERNAL resident process (e.g. an easyremote Python host running a
// generator) stream many frames into the daemon's Axon stream plane
// without re-spawning per frame — the gap that shell-exec (one bounded
// result, RPC-only) structurally cannot fill.
//
// The handler is synchronous (the `LocalStreamHandler` contract): it
// builds a `broadcast` channel, spawns a reader task, and returns
// `StreamSource::Live(rx)` immediately. A failure to *open* (no runtime
// to spawn on) is the only `Err` path — once the source is returned the
// stream is live and every terminal outcome travels as an in-band frame
// (§I3: a failed open never produces a half-live session; a live stream
// always reaches an explicit terminal, never a silent drop).
//
// Wire protocol: the single source of truth is the doc comment on
// `crate::core::ability_spec::HostStreamExec`. Summary, with the five
// load-bearing invariants enforced here:
//
//   1. `seq` strictly increasing from 0 (reorder/gap = truncation).
//   2. rolling `output_hash = H(prev || seq || canonical_json(frame))`,
//      verified against the host's `terminal.output_hash`.
//   3. `terminal` / `error` mutually exclusive, each at most once.
//   4. EOF before terminal/error = STREAM_TRUNCATED (not clean).
//   5. caller seven-tuple comes from the envelope-aware handler, never
//      from args (see register site in chat_ability.rs).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

use crate::core::ability_spec::HostStreamExec;
use crate::runtime::ability::canonical_json_bytes;
use crate::runtime::ability_dispatch::StreamSource;

/// Broadcast depth. Frames are small JSON values; a generous buffer
/// absorbs a slow consumer without forcing the reader task to block on
/// the socket. Lag (overflow) is surfaced by the runtime forwarder, not
/// silently dropped.
const FRAME_CHANNEL_DEPTH: usize = 256;

/// Open a `host_stream` ability against its external warm host and
/// return a live frame source.
///
/// `call_id` is the runtime invocation id used purely to correlate the
/// request line on the wire; it is not protocol identity.
pub fn run_host_stream(
    spec: &HostStreamExec,
    args: &Value,
    call_id: &str,
    caller: &str,
) -> anyhow::Result<StreamSource> {
    let (tx, rx) = broadcast::channel::<Value>(FRAME_CHANNEL_DEPTH);

    let socket_path = spec.host_socket.clone();
    // `caller` + `call_id` let a Context-taking host function read who
    // invoked it (read-only identity injection); the warm host builds a
    // Context from them. They are envelope projections, not new tuple
    // fields. Child-call composition (ctx.call) needs the parent receipt
    // URA too — a separate enabler — so it is not carried here yet.
    let request = json!({
        "request": {
            "fn": spec.function,
            "args": args,
            "call_id": call_id,
            "caller": caller,
        }
    });

    let handle = tokio::runtime::Handle::try_current()
        .map_err(|err| anyhow::anyhow!("host_stream open requires a Tokio runtime: {err}"))?;
    handle.spawn(async move {
        if let Err(err) = pump_host_stream(&socket_path, &request, &tx).await {
            // Any failure after the source went live travels in-band as
            // a single terminal error frame, so the consumer always
            // observes an explicit terminal (invariant 4 routes here for
            // truncation; connect/IO failures route here too). A send
            // error means the receiver is already gone — nothing to do.
            let _ = tx.send(error_frame(&err));
        }
        // Dropping `tx` here closes the channel; the runtime forwarder
        // turns that into the terminal AbilityFrame. The error frame
        // above (if any) is delivered first because send is ordered.
    });

    Ok(StreamSource::Live(rx))
}

/// Drive one host-stream session to a clean terminal, or return the
/// structured failure that becomes the terminal error frame.
async fn pump_host_stream(
    socket_path: &str,
    request: &Value,
    tx: &broadcast::Sender<Value>,
) -> Result<(), StreamFailure> {
    let stream = UnixStream::connect(socket_path).await.map_err(|e| {
        StreamFailure::new(
            StreamFailureKind::HostUnreachable,
            format!("connect {socket_path}: {e}"),
        )
    })?;
    let (read_half, mut write_half) = stream.into_split();

    let line = serde_json::to_string(request).map_err(|e| {
        StreamFailure::new(StreamFailureKind::Internal, format!("encode request: {e}"))
    })?;
    write_half
        .write_all(format!("{line}\n").as_bytes())
        .await
        .map_err(|e| {
            StreamFailure::new(
                StreamFailureKind::HostUnreachable,
                format!("send request: {e}"),
            )
        })?;
    write_half.flush().await.map_err(|e| {
        StreamFailure::new(
            StreamFailureKind::HostUnreachable,
            format!("flush request: {e}"),
        )
    })?;

    let mut reader = BufReader::new(read_half).lines();
    let mut next_seq: u64 = 0;
    let mut rolling = RollingHash::new();

    while let Some(line) = reader.next_line().await.map_err(|e| {
        StreamFailure::new(
            StreamFailureKind::StreamTruncated,
            format!("read frame: {e}"),
        )
    })? {
        if line.trim().is_empty() {
            continue;
        }
        let frame: Value = serde_json::from_str(&line).map_err(|e| {
            StreamFailure::new(
                StreamFailureKind::Protocol,
                format!("frame is not JSON: {e}"),
            )
        })?;

        match decode_host_frame(&frame)? {
            HostFrame::StreamItem { seq, item } => {
                if seq != next_seq {
                    // Invariant 1: a gap or reorder must not be invisible.
                    return Err(StreamFailure::new(
                        StreamFailureKind::StreamTruncated,
                        format!("frame reorder/gap: expected seq {next_seq}, got {seq}"),
                    ));
                }
                rolling.fold(seq, item);
                next_seq += 1;
                // A lagging/absent consumer (`send` error) ends the session;
                // there is no point reading frames nobody will receive.
                if tx.send(item.clone()).is_err() {
                    return Ok(());
                }
            }
            HostFrame::Terminal(terminal) => {
                // Invariant 3 (terminal/error mutually exclusive, each at
                // most once) holds structurally: the first terminal/error
                // returns out of the loop, so a second can never be read.
                verify_terminal(terminal, &rolling, next_seq)?;
                return Ok(()); // clean end-of-stream
            }
            HostFrame::Error(error) => {
                return Err(StreamFailure::from_host_error(error));
            }
        }
    }

    // Invariant 4: the socket closed before terminal/error arrived.
    Err(StreamFailure::new(
        StreamFailureKind::StreamTruncated,
        "host closed the connection before sending terminal/error".to_string(),
    ))
}

#[derive(Debug)]
enum HostFrame<'a> {
    StreamItem { seq: u64, item: &'a Value },
    Terminal(&'a Value),
    Error(&'a Value),
}

fn decode_host_frame(frame: &Value) -> Result<HostFrame<'_>, StreamFailure> {
    let has_item = frame.get("stream_item").is_some();
    let has_terminal = frame.get("terminal").is_some();
    let has_error = frame.get("error").is_some();
    let kinds = usize::from(has_item) + usize::from(has_terminal) + usize::from(has_error);
    if kinds != 1 {
        return Err(StreamFailure::new(
            StreamFailureKind::Protocol,
            "frame must contain exactly one of stream_item/terminal/error".to_string(),
        ));
    }

    if let Some(item) = frame.get("stream_item") {
        let seq = frame.get("seq").and_then(Value::as_u64).ok_or_else(|| {
            StreamFailure::new(
                StreamFailureKind::Protocol,
                "stream_item missing u64 seq".into(),
            )
        })?;
        return Ok(HostFrame::StreamItem { seq, item });
    }
    if let Some(terminal) = frame.get("terminal") {
        return Ok(HostFrame::Terminal(terminal));
    }
    let error = frame
        .get("error")
        .expect("exactly one host-stream frame kind was already checked");
    Ok(HostFrame::Error(error))
}

/// Recompute the rolling hash and frame count against the host's
/// declared terminal, so a truncated/altered stream cannot masquerade
/// as a clean terminal (invariant 2).
fn verify_terminal(
    terminal: &Value,
    rolling: &RollingHash,
    frames_seen: u64,
) -> Result<(), StreamFailure> {
    let frames = terminal
        .get("frames")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            StreamFailure::new(
                StreamFailureKind::Protocol,
                "terminal missing u64 frames".to_string(),
            )
        })?;
    if frames != frames_seen {
        return Err(StreamFailure::new(
            StreamFailureKind::StreamTruncated,
            format!("terminal frame count {frames} != frames received {frames_seen}"),
        ));
    }

    let declared = terminal
        .get("output_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            StreamFailure::new(
                StreamFailureKind::Protocol,
                "terminal missing string output_hash".to_string(),
            )
        })?;
    let computed = rolling.finish();
    if declared != computed {
        return Err(StreamFailure::new(
            StreamFailureKind::StreamTruncated,
            format!("output_hash mismatch: host {declared} != computed {computed}"),
        ));
    }
    Ok(())
}

/// `output_hash = H(prev_hash || seq || canonical_json(frame))` folded
/// over every emitted frame in `seq` order, seeded from the empty hash.
/// Canonical JSON is the shared daemon helper used by descriptor hashing;
/// host-stream rolling hashes must not maintain a second JSON normalizer.
struct RollingHash {
    prev: [u8; 32],
}

impl RollingHash {
    fn new() -> Self {
        Self {
            prev: Sha256::digest(b"").into(),
        }
    }

    fn fold(&mut self, seq: u64, frame: &Value) {
        let mut hasher = Sha256::new();
        hasher.update(self.prev);
        hasher.update(seq.to_be_bytes());
        hasher.update(canonical_json_bytes(frame));
        self.prev = hasher.finalize().into();
    }

    fn finish(&self) -> String {
        format!("sha256:{}", hex::encode(self.prev))
    }
}

/// The protocol-level reason a host stream failed. The wire codes are a
/// fixed, enumerable set, so they are a single typed vocabulary here
/// rather than bare string literals scattered across the executor. The
/// only open-ended case is a kind echoed verbatim from a host's own error
/// frame, which `Host` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamFailureKind {
    /// Could not reach or talk to the host socket.
    HostUnreachable,
    /// The stream ended, reordered, or hashed differently than declared.
    StreamTruncated,
    /// A frame violated the wire contract (missing/wrong-typed fields).
    Protocol,
    /// A failure originating inside the executor (e.g. request encoding).
    Internal,
    /// A kind reported by the host in its own error frame, adopted
    /// verbatim. Defaults to `HOST_ERROR` when the host omits one.
    Host(String),
}

impl StreamFailureKind {
    fn as_str(&self) -> &str {
        match self {
            StreamFailureKind::HostUnreachable => "HOST_UNREACHABLE",
            StreamFailureKind::StreamTruncated => "STREAM_TRUNCATED",
            StreamFailureKind::Protocol => "PROTOCOL",
            StreamFailureKind::Internal => "INTERNAL",
            StreamFailureKind::Host(kind) => kind.as_str(),
        }
    }
}

/// A structured stream failure, projected onto the in-band error frame
/// the consumer receives as the stream's terminal.
#[derive(Debug)]
struct StreamFailure {
    kind: StreamFailureKind,
    message: String,
}

impl StreamFailure {
    fn new(kind: StreamFailureKind, message: String) -> Self {
        Self { kind, message }
    }

    /// Adopt a host-sent `{"error":{...}}` verbatim where present.
    fn from_host_error(error: &Value) -> Self {
        let kind = error
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("HOST_ERROR")
            .to_string();
        Self {
            kind: StreamFailureKind::Host(kind),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("host reported an error")
                .to_string(),
        }
    }
}

/// The in-band terminal error frame. `error` here is the ability frame
/// payload the runtime forwards; the runtime still emits its own
/// terminal marker when `tx` drops, so the consumer gets both the
/// reason and a definite end-of-stream.
fn error_frame(failure: &StreamFailure) -> Value {
    json!({
        "error": {
            "kind": failure.kind.as_str(),
            "message": failure.message,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_hash_is_order_sensitive_and_deterministic() {
        let mut a = RollingHash::new();
        a.fold(0, &json!({"x": 1}));
        a.fold(1, &json!({"y": 2}));

        let mut b = RollingHash::new();
        b.fold(0, &json!({"x": 1}));
        b.fold(1, &json!({"y": 2}));
        assert_eq!(
            a.finish(),
            b.finish(),
            "same frames in same order → same hash"
        );

        let mut c = RollingHash::new();
        c.fold(0, &json!({"y": 2}));
        c.fold(1, &json!({"x": 1}));
        assert_ne!(a.finish(), c.finish(), "reordered frames → different hash");
    }

    #[test]
    fn rolling_hash_uses_shared_key_sorted_json() {
        let mut a = RollingHash::new();
        a.fold(0, &json!({"b": 2, "a": {"d": 4, "c": 3}}));

        let mut b = RollingHash::new();
        b.fold(0, &json!({"a": {"c": 3, "d": 4}, "b": 2}));

        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn verify_terminal_rejects_frame_count_mismatch() {
        let rolling = RollingHash::new();
        let err = verify_terminal(
            &json!({"frames": 3, "output_hash": rolling.finish()}),
            &rolling,
            2,
        )
        .unwrap_err();
        assert_eq!(err.kind, StreamFailureKind::StreamTruncated);
    }

    #[test]
    fn verify_terminal_rejects_missing_frame_count() {
        let rolling = RollingHash::new();
        let err =
            verify_terminal(&json!({"output_hash": rolling.finish()}), &rolling, 0).unwrap_err();
        assert_eq!(err.kind, StreamFailureKind::Protocol);
    }

    #[test]
    fn verify_terminal_rejects_missing_output_hash() {
        let rolling = RollingHash::new();
        let err = verify_terminal(&json!({"frames": 0}), &rolling, 0).unwrap_err();
        assert_eq!(err.kind, StreamFailureKind::Protocol);
    }

    #[test]
    fn verify_terminal_rejects_output_hash_mismatch() {
        let mut rolling = RollingHash::new();
        rolling.fold(0, &json!({"x": 1}));
        let err = verify_terminal(
            &json!({"frames": 1, "output_hash": "sha256:deadbeef"}),
            &rolling,
            1,
        )
        .unwrap_err();
        assert_eq!(err.kind, StreamFailureKind::StreamTruncated);
    }

    #[test]
    fn verify_terminal_accepts_matching_hash_and_count() {
        let mut rolling = RollingHash::new();
        rolling.fold(0, &json!({"x": 1}));
        let good = rolling.finish();
        verify_terminal(&json!({"output_hash": good, "frames": 1}), &rolling, 1)
            .expect("matching terminal must verify");
    }

    #[test]
    fn decode_host_frame_rejects_mixed_frame_kinds() {
        let err = decode_host_frame(&json!({
            "seq": 0,
            "stream_item": {"x": 1},
            "terminal": {"frames": 1, "output_hash": "sha256:deadbeef"}
        }))
        .unwrap_err();
        assert_eq!(err.kind, StreamFailureKind::Protocol);
    }

    #[test]
    fn host_error_frame_preserves_kind_and_message() {
        let f = StreamFailure::from_host_error(&json!({
            "kind": "BOOM", "message": "it broke"
        }));
        let frame = error_frame(&f);
        assert_eq!(frame["error"]["kind"], "BOOM");
        assert_eq!(frame["error"]["message"], "it broke");
    }
}
