// EasyNet CLI — remote desktop transport state
// =============================================
//
// File: src/plugins/builtin/remote_desktop/session_transport_state.rs
// Description: Production media and diagnostic preview transport state.

use serde_json::Value;
use tokio::sync::watch;

/// Latest media pipeline statistics captured from the media transport.
///
/// This is a domain wrapper around the current JSON telemetry shape. The media
/// pipeline owns the payload schema; session state stores it explicitly as
/// telemetry rather than as an unlabelled JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopMediaStats {
    value: Value,
}

impl RemoteDesktopMediaStats {
    #[cfg(target_os = "macos")]
    fn new(value: Value) -> Self {
        Self { value }
    }

    pub(in crate::plugins::builtin::remote_desktop) fn to_value(&self) -> Value {
        self.value.clone()
    }
}

/// Transport facts owned by one remote desktop session.
///
/// Lifecycle permission stays in `RemoteDesktopSession`; this type only keeps
/// mutable transport state and makes readiness/preview transitions explicit.
#[derive(Debug, Clone)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopTransportState {
    media_transport_ready: bool,
    media_stats: Option<RemoteDesktopMediaStats>,
    preview_attached: bool,
    preview_stop_tx: Option<watch::Sender<bool>>,
}

impl RemoteDesktopTransportState {
    pub(in crate::plugins::builtin::remote_desktop) fn new() -> Self {
        Self {
            media_transport_ready: false,
            media_stats: None,
            preview_attached: false,
            preview_stop_tx: None,
        }
    }

    pub(in crate::plugins::builtin::remote_desktop) fn media_transport_ready(&self) -> bool {
        self.media_transport_ready
    }

    pub(in crate::plugins::builtin::remote_desktop) fn media_stats(&self) -> Option<Value> {
        self.media_stats
            .as_ref()
            .map(RemoteDesktopMediaStats::to_value)
    }

    pub(in crate::plugins::builtin::remote_desktop) fn preview_attached(&self) -> bool {
        self.preview_attached
    }

    pub(in crate::plugins::builtin::remote_desktop) fn attach_preview_transport(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) -> Option<watch::Sender<bool>> {
        self.preview_attached = true;
        self.preview_stop_tx.replace(stop_tx)
    }

    pub(in crate::plugins::builtin::remote_desktop) fn mark_media_pending(&mut self) {
        self.media_transport_ready = false;
    }

    pub(in crate::plugins::builtin::remote_desktop) fn mark_media_ready(&mut self) -> bool {
        if self.media_transport_ready {
            return false;
        }
        self.media_transport_ready = true;
        true
    }

    #[cfg(target_os = "macos")]
    pub(in crate::plugins::builtin::remote_desktop) fn record_media_stats(&mut self, stats: Value) {
        self.media_stats = Some(RemoteDesktopMediaStats::new(stats));
    }

    pub(in crate::plugins::builtin::remote_desktop) fn detach_preview_transport(
        &mut self,
    ) -> Option<watch::Sender<bool>> {
        self.preview_attached = false;
        self.media_transport_ready = false;
        self.preview_stop_tx.take()
    }

    #[cfg(test)]
    pub(in crate::plugins::builtin::remote_desktop) fn install_preview_transport_for_test(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) {
        self.preview_attached = true;
        self.preview_stop_tx = Some(stop_tx);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::sync::watch;

    use super::RemoteDesktopTransportState;

    #[test]
    fn remote_desktop_transport_ready_transition_is_idempotent() {
        let mut transport = RemoteDesktopTransportState::new();

        assert!(transport.mark_media_ready());
        assert!(!transport.mark_media_ready());
        assert!(transport.media_transport_ready());

        transport.mark_media_pending();
        assert!(!transport.media_transport_ready());
    }

    #[test]
    fn remote_desktop_transport_detach_clears_preview_and_media_readiness() {
        let mut transport = RemoteDesktopTransportState::new();
        let (stop_tx, _stop_rx) = watch::channel(false);

        assert!(transport.attach_preview_transport(stop_tx).is_none());
        assert!(transport.mark_media_ready());
        assert!(transport.preview_attached());
        assert!(transport.detach_preview_transport().is_some());

        assert!(!transport.preview_attached());
        assert!(!transport.media_transport_ready());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn remote_desktop_transport_records_latest_media_stats() {
        let mut transport = RemoteDesktopTransportState::new();

        transport.record_media_stats(json!({ "frames": 1 }));
        transport.record_media_stats(json!({ "frames": 2 }));

        assert_eq!(transport.media_stats(), Some(json!({ "frames": 2 })));
    }
}
