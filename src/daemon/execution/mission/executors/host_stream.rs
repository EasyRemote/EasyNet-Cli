// EasyNet CLI — host_stream executor
// =================================================================
//
// File: src/daemon/execution/mission/executors/host_stream.rs
//
// Framed external-host executor for the `host_stream` AbilityExec. It lets
// an external resident process serve either a bounded unary result or a
// generator stream without re-spawning per frame. The manifest admission
// action selects which canonical runtime geometry consumes this transport.
//
// The stream handler is synchronous (the `LocalStreamHandler` contract): it
// builds a `broadcast` channel, spawns a reader task, and returns
// `StreamSource::Live(rx)` immediately. A failure to *open* (no runtime
// to spawn on) is the only `Err` path — once the source is returned the
// stream is live and every terminal outcome travels as an in-band frame
// (§I3: a failed open never produces a half-live session; a live stream
// always reaches an explicit terminal, never a silent drop).
//
// Wire protocol: the single source of truth is the doc comment on
// `crate::daemon::ability::manifest::HostStreamExec`. Summary, with the five
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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

mod contract;

use self::contract::{
    decode_host_frame, verify_terminal, HostFrame, HostStreamFailure, HostStreamFailureKind,
    HostStreamHashState,
};
use crate::daemon::ability::dispatch::StreamSource;
use crate::daemon::ability::manifest::HostStreamExec;

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
    let request = host_stream_request(spec, args, call_id, caller);

    let handle = tokio::runtime::Handle::try_current()
        .map_err(|err| anyhow::anyhow!("host_stream open requires a Tokio runtime: {err}"))?;
    handle.spawn(async move {
        let mut sink = HostStreamSink::Broadcast(&tx);
        if let Err(err) = pump_host_stream(&socket_path, &request, &mut sink).await {
            // Any failure after the source went live travels in-band as
            // a single terminal error frame, so the consumer always
            // observes an explicit terminal (invariant 4 routes here for
            // truncation; connect/IO failures route here too). A send
            // error means the receiver is already gone — nothing to do.
            let _ = tx.send(err.error_frame());
        }
        // Dropping `tx` here closes the channel; the runtime forwarder
        // turns that into the terminal AbilityFrame. The error frame
        // above (if any) is delivered first because send is ordered.
    });

    Ok(StreamSource::Live(rx))
}

/// Invoke the same external host transport as a unary RPC. The host wire is
/// still framed because that is the transport contract, but RPC geometry
/// requires exactly one successful item and a verified terminal. Stream
/// items are collected only for this bounded unary adapter; generators never
/// select this path.
pub fn run_host_stream_unary(
    spec: &HostStreamExec,
    args: &Value,
    call_id: &str,
    caller: &str,
) -> anyhow::Result<Value> {
    let socket_path = spec.host_socket.clone();
    let request = host_stream_request(spec, args, call_id, caller);
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|err| anyhow::anyhow!("host_stream unary requires a Tokio runtime: {err}"))?;
    handle.block_on(async move {
        let mut frames = Vec::with_capacity(1);
        let mut sink = HostStreamSink::Unary(&mut frames);
        pump_host_stream(&socket_path, &request, &mut sink)
            .await
            .map_err(|err| anyhow::anyhow!("host_stream unary failed: {}", err.message))?;
        match frames.len() {
            1 => Ok(frames
                .pop()
                .expect("one frame was reported by the length check")),
            0 => Err(anyhow::anyhow!(
                "host_stream unary completed without a result frame"
            )),
            count => Err(anyhow::anyhow!(
                "host_stream unary returned {count} result frames"
            )),
        }
    })
}

fn host_stream_request(spec: &HostStreamExec, args: &Value, call_id: &str, caller: &str) -> Value {
    json!({
        "request": {
            "fn": spec.function,
            "args": args,
            "call_id": call_id,
            "caller": caller,
        }
    })
}

enum HostStreamSink<'a> {
    Broadcast(&'a broadcast::Sender<Value>),
    Unary(&'a mut Vec<Value>),
}

/// Drive one host-stream session to a clean terminal, or return the
/// structured failure that becomes the terminal error frame.
async fn pump_host_stream(
    socket_path: &str,
    request: &Value,
    sink: &mut HostStreamSink<'_>,
) -> Result<(), HostStreamFailure> {
    let stream = UnixStream::connect(socket_path).await.map_err(|e| {
        HostStreamFailure::new(
            HostStreamFailureKind::HostUnreachable,
            format!("connect {socket_path}: {e}"),
        )
    })?;
    let (read_half, mut write_half) = stream.into_split();

    let line = serde_json::to_string(request).map_err(|e| {
        HostStreamFailure::new(
            HostStreamFailureKind::Internal,
            format!("encode request: {e}"),
        )
    })?;
    write_half
        .write_all(format!("{line}\n").as_bytes())
        .await
        .map_err(|e| {
            HostStreamFailure::new(
                HostStreamFailureKind::HostUnreachable,
                format!("send request: {e}"),
            )
        })?;
    write_half.flush().await.map_err(|e| {
        HostStreamFailure::new(
            HostStreamFailureKind::HostUnreachable,
            format!("flush request: {e}"),
        )
    })?;

    let mut reader = BufReader::new(read_half).lines();
    let mut next_seq: u64 = 0;
    let mut rolling = HostStreamHashState::new();

    while let Some(line) = reader.next_line().await.map_err(|e| {
        HostStreamFailure::new(
            HostStreamFailureKind::StreamTruncated,
            format!("read frame: {e}"),
        )
    })? {
        if line.trim().is_empty() {
            continue;
        }
        let frame: Value = serde_json::from_str(&line).map_err(|e| {
            HostStreamFailure::new(
                HostStreamFailureKind::Protocol,
                format!("frame is not JSON: {e}"),
            )
        })?;

        match decode_host_frame(&frame)? {
            HostFrame::StreamItem { seq, item } => {
                if seq != next_seq {
                    // Invariant 1: a gap or reorder must not be invisible.
                    return Err(HostStreamFailure::new(
                        HostStreamFailureKind::StreamTruncated,
                        format!("frame reorder/gap: expected seq {next_seq}, got {seq}"),
                    ));
                }
                rolling.fold_item(seq, item)?;
                next_seq += 1;
                match sink {
                    // A lagging/absent consumer (`send` error) ends the
                    // session; there is no point reading frames nobody will
                    // receive.
                    HostStreamSink::Broadcast(tx) if tx.send(item.clone()).is_err() => {
                        return Ok(())
                    }
                    HostStreamSink::Broadcast(_) => {}
                    HostStreamSink::Unary(frames) if !frames.is_empty() => {
                        return Err(HostStreamFailure::new(
                            HostStreamFailureKind::Protocol,
                            "unary host_stream returned more than one result frame".to_string(),
                        ))
                    }
                    HostStreamSink::Unary(frames) => frames.push(item.clone()),
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
                return Err(HostStreamFailure::from_host_error(error));
            }
        }
    }

    // Invariant 4: the socket closed before terminal/error arrived.
    Err(HostStreamFailure::new(
        HostStreamFailureKind::StreamTruncated,
        "host closed the connection before sending terminal/error".to_string(),
    ))
}
