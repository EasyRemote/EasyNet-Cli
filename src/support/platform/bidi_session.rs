// EasyNet CLI — live local-daemon InvokeBidi session
// ==================================================
//
// File: src/support/platform/bidi_session.rs
// Description: One live, bounded upstream/downstream transport session for
//              callers attached to the local easynet-daemon.
//
// Protocol Responsibility
// -----------------------
// Owns only Axon InvokeBidi frame sequencing and the local gRPC stream. Route
// selection, invocation tuple construction, authority issuance, and business
// frame interpretation remain with their respective issuer/product layers.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use axon_sdk::pb::axon::v1::{
    bidi_control, invoke_bidi_up::Payload as UpPayload, BidiControl, BinaryChunk, EnvelopeOpen,
    InvokeBidiDown, InvokeBidiUp, PtyResize,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::support::platform::local_invoke::LocalBidiFrame;

const UPSTREAM_CHANNEL_BOUND: usize = 32;

/// Error labels for one daemon-attached Bidi session.
///
/// The labels are diagnostic facts only; they never participate in routing or
/// authority decisions.
#[derive(Debug, Clone)]
pub(crate) struct DaemonBidiContext {
    invocation: String,
    execution_target: Option<String>,
}

impl DaemonBidiContext {
    pub(crate) fn local(invocation: impl Into<String>) -> Self {
        Self {
            invocation: invocation.into(),
            execution_target: None,
        }
    }

    pub(crate) fn remote(
        invocation: impl Into<String>,
        execution_target: impl Into<String>,
    ) -> Self {
        Self {
            invocation: invocation.into(),
            execution_target: Some(execution_target.into()),
        }
    }

    fn status_error(&self, phase: &str, status: tonic::Status) -> anyhow::Error {
        let target = self
            .execution_target
            .as_deref()
            .map(|value| format!(" for target `{value}`"))
            .unwrap_or_default();
        anyhow::anyhow!(
            "{} `{}`{} failed (code={:?}): {}",
            phase,
            self.invocation,
            target,
            status.code(),
            status.message(),
        )
    }
}

/// Live upstream half. Sequence ownership stays here so product callers cannot
/// emit duplicate or reordered Axon frames.
pub(crate) struct DaemonBidiSender {
    context: DaemonBidiContext,
    upstream: mpsc::Sender<InvokeBidiUp>,
    next_sequence: u64,
    eof_sent: bool,
}

impl DaemonBidiSender {
    pub(crate) async fn send_control_json(
        &mut self,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let data = serde_json::to_vec(value)
            .with_context(|| format!("encode {} PTY control frame", self.context.invocation))?;
        self.send_payload(UpPayload::BinaryChunk(BinaryChunk {
            stream_id: crate::daemon::ability::wire::CONTROL_STREAM_ID,
            data,
            ..BinaryChunk::default()
        }))
        .await
    }

    pub(crate) async fn send_pty_control_json(
        &mut self,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.send_control_json(value).await
    }

    pub(crate) async fn send_pty_resize(&mut self, cols: u32, rows: u32) -> anyhow::Result<()> {
        self.send_payload(UpPayload::Control(BidiControl {
            control: Some(bidi_control::Control::PtyResize(PtyResize { cols, rows })),
        }))
        .await
    }

    pub(crate) async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        self.send_payload(UpPayload::BinaryChunk(BinaryChunk {
            stream_id: 1,
            data,
            ..BinaryChunk::default()
        }))
        .await
    }

    pub(crate) async fn send_eof(&mut self) -> anyhow::Result<()> {
        if self.eof_sent {
            return Ok(());
        }
        self.send_payload(UpPayload::Control(BidiControl {
            control: Some(bidi_control::Control::Eof(true)),
        }))
        .await?;
        self.eof_sent = true;
        Ok(())
    }

    async fn send_payload(&mut self, payload: UpPayload) -> anyhow::Result<()> {
        if self.eof_sent {
            anyhow::bail!(
                "InvokeBidi upstream for `{}` is already closed",
                self.context.invocation
            );
        }
        let sequence = self.next_sequence;
        self.upstream
            .send(InvokeBidiUp {
                sequence,
                mac: Vec::new(),
                payload: Some(payload),
            })
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "InvokeBidi upstream for `{}` closed before frame {sequence}",
                    self.context.invocation
                )
            })?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(())
    }
}

/// Live downstream half. Internal dispatch frames are consumed here and never
/// leaked into product frame protocols.
pub(crate) struct DaemonBidiReceiver {
    context: DaemonBidiContext,
    downstream: tonic::Streaming<InvokeBidiDown>,
    terminal_seen: bool,
}

impl DaemonBidiReceiver {
    pub(crate) async fn recv(&mut self) -> anyhow::Result<Option<LocalBidiFrame>> {
        if self.terminal_seen {
            return Ok(None);
        }
        loop {
            let Some(frame) = self
                .downstream
                .message()
                .await
                .map_err(|status| self.context.status_error("read InvokeBidi", status))?
            else {
                return Ok(None);
            };
            let Some(projected) =
                crate::support::platform::local_invoke::project_invoke_bidi_down_frame(frame)?
            else {
                continue;
            };
            self.terminal_seen = projected.terminal;
            return Ok(Some(projected));
        }
    }
}

pub(crate) struct DaemonBidiSession {
    sender: DaemonBidiSender,
    receiver: DaemonBidiReceiver,
}

impl DaemonBidiSession {
    pub(crate) fn split(self) -> (DaemonBidiSender, DaemonBidiReceiver) {
        (self.sender, self.receiver)
    }
}

/// Submit frame zero and keep both stream halves live for the product caller.
pub(crate) async fn open_daemon_bidi_session(
    socket_path: PathBuf,
    timeout: Duration,
    context: DaemonBidiContext,
    envelope_open: EnvelopeOpen,
    open_mac: Vec<u8>,
) -> anyhow::Result<DaemonBidiSession> {
    let channel = crate::support::platform::local_daemon_grpc::connect_channel(
        socket_path.clone(),
        timeout,
        Duration::from_secs(10),
    )
    .await
    .with_context(|| {
        format!(
            "connect to local daemon InvokeBidi endpoint at {}",
            socket_path.display()
        )
    })?;
    let mut client = crate::daemon::invocation::transport::invocation_client(channel);
    let (upstream, receiver) = mpsc::channel::<InvokeBidiUp>(UPSTREAM_CHANNEL_BOUND);
    upstream
        .send(InvokeBidiUp {
            sequence: 0,
            mac: open_mac,
            payload: Some(UpPayload::EnvelopeOpen(envelope_open)),
        })
        .await
        .map_err(|_| anyhow::anyhow!("InvokeBidi upstream closed before frame 0"))?;

    let downstream = client
        .invoke_bidi(tonic::Request::new(ReceiverStream::new(receiver)))
        .await
        .map_err(|status| context.status_error("open InvokeBidi", status))?
        .into_inner();

    Ok(DaemonBidiSession {
        sender: DaemonBidiSender {
            context: context.clone(),
            upstream,
            next_sequence: 1,
            eof_sent: false,
        },
        receiver: DaemonBidiReceiver {
            context,
            downstream,
            terminal_seen: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sender() -> (DaemonBidiSender, mpsc::Receiver<InvokeBidiUp>) {
        let (upstream, receiver) = mpsc::channel(4);
        (
            DaemonBidiSender {
                context: DaemonBidiContext::local("terminal.attach"),
                upstream,
                next_sequence: 1,
                eof_sent: false,
            },
            receiver,
        )
    }

    #[tokio::test]
    async fn sender_owns_strict_sequence_across_control_binary_and_eof() {
        let (mut sender, mut receiver) = test_sender();
        sender
            .send_control_json(&serde_json::json!({"type": "resize", "cols": 80, "rows": 24}))
            .await
            .expect("json frame");
        sender.send_binary(vec![2, 3]).await.expect("binary frame");
        sender.send_eof().await.expect("eof");
        sender.send_eof().await.expect("idempotent eof");

        let first = receiver.recv().await.expect("first frame");
        let second = receiver.recv().await.expect("second frame");
        let third = receiver.recv().await.expect("eof frame");
        assert_eq!((first.sequence, second.sequence, third.sequence), (1, 2, 3));
        assert!(matches!(first.payload, Some(UpPayload::BinaryChunk(_))));
        assert!(matches!(second.payload, Some(UpPayload::BinaryChunk(_))));
        assert!(matches!(third.payload, Some(UpPayload::Control(_))));
        assert!(receiver.try_recv().is_err(), "idempotent EOF emitted twice");
    }

    #[tokio::test]
    async fn sender_rejects_business_frames_after_eof() {
        let (mut sender, _receiver) = test_sender();
        sender.send_eof().await.expect("eof");
        let error = sender
            .send_binary(vec![1])
            .await
            .expect_err("frame after EOF must fail");
        assert!(error.to_string().contains("already closed"));
    }
}
