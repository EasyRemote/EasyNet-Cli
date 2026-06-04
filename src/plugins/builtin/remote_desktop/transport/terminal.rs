// EasyNet CLI — remote desktop terminal guards
// =============================================
//
// File: src/plugins/builtin/remote_desktop/transport/terminal.rs
// Description: Idempotent terminal-frame guards for stream and bidi calls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc;

use crate::runtime::ability_dispatch::BidiOutputFrame;

/// Single-terminal guard for one remote desktop Bidi invocation.
///
/// Invariant 1: every clone shares one atomic terminal bit.
/// Invariant 2: once any producer emits a `{"type":"closed"}` frame, later
/// producers silently suppress their terminal frame.
#[derive(Clone, Default)]
pub(in crate::plugins::builtin::remote_desktop) struct BidiTerminalGuard {
    sent: Arc<AtomicBool>,
}

impl BidiTerminalGuard {
    pub(in crate::plugins::builtin::remote_desktop) fn new() -> Self {
        Self::default()
    }

    pub(in crate::plugins::builtin::remote_desktop) async fn send_closed(
        &self,
        to_client: &mpsc::Sender<BidiOutputFrame>,
        reason: &'static str,
    ) {
        if self.sent.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = to_client
            .send(BidiOutputFrame::json(json!({
                "type": "closed",
                "reason": reason,
            })))
            .await;
    }

    pub(in crate::plugins::builtin::remote_desktop) fn send_blocking_closed(
        &self,
        to_client: &mpsc::Sender<BidiOutputFrame>,
        reason: &'static str,
    ) {
        if self.sent.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = to_client.blocking_send(BidiOutputFrame::json(json!({
            "type": "closed",
            "reason": reason,
        })));
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
}
