// EasyNet CLI — remote desktop terminal guards
// =============================================
//
// File: plugins/remote-desktop/src/transport/terminal.rs
// Description: Idempotent terminal-frame guards for stream and bidi calls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;

use crate::daemon::ability::dispatch::BidiOutputFrame;

const BIDI_TERMINAL_SEND_DEADLINE: Duration = Duration::from_millis(250);

/// Single-terminal guard for one remote desktop Bidi invocation.
///
/// Invariant 1: every clone shares one atomic terminal bit.
/// Invariant 2: once any producer emits a terminal `closed` or `error` frame,
/// later producers silently suppress their terminal frame.
#[derive(Clone, Default)]
pub(in crate::daemon::plugins::remote_desktop) struct BidiTerminalGuard {
    sent: Arc<AtomicBool>,
}

impl BidiTerminalGuard {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self::default()
    }

    pub(in crate::daemon::plugins::remote_desktop) async fn send_closed(
        &self,
        to_client: &mpsc::Sender<BidiOutputFrame>,
        reason: &'static str,
    ) -> bool {
        if self.sent.swap(true, Ordering::AcqRel) {
            return false;
        }
        matches!(
            tokio::time::timeout(
                BIDI_TERMINAL_SEND_DEADLINE,
                to_client.send(BidiOutputFrame::json(json!({
                    "type": "closed",
                    "reason": reason,
                }))),
            )
            .await,
            Ok(Ok(()))
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn send_blocking_closed(
        &self,
        to_client: &mpsc::Sender<BidiOutputFrame>,
        reason: &'static str,
    ) -> bool {
        if self.sent.swap(true, Ordering::AcqRel) {
            return false;
        }
        to_client
            .try_send(BidiOutputFrame::json(json!({
                "type": "closed",
                "reason": reason,
            })))
            .is_ok()
    }

    pub(in crate::daemon::plugins::remote_desktop) async fn send_error(
        &self,
        to_client: &mpsc::Sender<BidiOutputFrame>,
        code: &'static str,
        message: impl Into<String>,
    ) -> bool {
        if self.sent.swap(true, Ordering::AcqRel) {
            return false;
        }
        matches!(
            tokio::time::timeout(
                BIDI_TERMINAL_SEND_DEADLINE,
                to_client.send(BidiOutputFrame::json(json!({
                    "type": "error",
                    "code": code,
                    "message": message.into(),
                }))),
            )
            .await,
            Ok(Ok(()))
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn send_blocking_error(
        &self,
        to_client: &mpsc::Sender<BidiOutputFrame>,
        code: &'static str,
        message: impl Into<String>,
    ) -> bool {
        if self.sent.swap(true, Ordering::AcqRel) {
            return false;
        }
        to_client
            .try_send(BidiOutputFrame::json(json!({
                "type": "error",
                "code": code,
                "message": message.into(),
            })))
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bidi_terminal_guard_emits_one_closed_frame() {
        let guard = BidiTerminalGuard::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);

        guard.send_closed(&tx, "first").await;
        guard.send_closed(&tx, "second").await;
        drop(tx);

        let first = rx
            .recv()
            .await
            .expect("first terminal frame emitted")
            .into_json_value()
            .expect("terminal frame is JSON");
        assert_eq!(
            first.get("type").and_then(serde_json::Value::as_str),
            Some("closed")
        );
        assert_eq!(
            first.get("reason").and_then(serde_json::Value::as_str),
            Some("first")
        );
        assert!(
            rx.recv().await.is_none(),
            "second terminal frame must be suppressed"
        );
    }

    #[tokio::test]
    async fn bidi_terminal_guard_error_suppresses_followup_closed_frame() {
        let guard = BidiTerminalGuard::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);

        assert!(
            guard
                .send_error(&tx, "capture_failed", "encoder stopped")
                .await
        );
        assert!(!guard.send_closed(&tx, "preview_client_closed").await);
        drop(tx);

        let first = rx
            .recv()
            .await
            .expect("error terminal frame emitted")
            .into_json_value()
            .expect("terminal frame is JSON");
        assert_eq!(
            first.get("type").and_then(serde_json::Value::as_str),
            Some("error")
        );
        assert_eq!(
            first.get("code").and_then(serde_json::Value::as_str),
            Some("capture_failed")
        );
        assert!(
            rx.recv().await.is_none(),
            "closed frame must be suppressed after terminal error"
        );
    }

    #[tokio::test]
    async fn bidi_terminal_guard_does_not_block_shutdown_on_full_client_queue() {
        let guard = BidiTerminalGuard::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        tx.send(BidiOutputFrame::json(json!({"type": "frame"})))
            .await
            .expect("test queue fills");

        let started = tokio::time::Instant::now();
        assert!(
            !guard.send_closed(&tx, "session_closing").await,
            "full client queue cannot claim terminal frame delivery"
        );
        assert!(
            started.elapsed() <= BIDI_TERMINAL_SEND_DEADLINE + Duration::from_millis(100),
            "client backpressure must not own worker settlement"
        );
    }
}
