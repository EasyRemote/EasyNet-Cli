// EasyNet CLI — remote desktop transport state
// =============================================
//
// File: plugins/remote-desktop/src/session_transport_state.rs
// Description: Orthogonal production-media and diagnostic-preview state.
//
// Protocol Responsibility:
// - None. This is plugin-owned product lifecycle state.
//
// Implementation Approach:
// - Scope every production-media fact to a monotonic transport epoch.
// - Keep diagnostic preview attachment independent from production readiness.
//
// Usage Contract:
// - Asynchronous transport callbacks must supply their epoch. Stale epochs are
//   ignored without mutating the current session generation.
//
// Architectural Position:
// - Remote-desktop session aggregate component.

use serde_json::{json, Value};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::daemon::plugins::remote_desktop) struct TransportEpoch(u64);

impl TransportEpoch {
    pub(in crate::daemon::plugins::remote_desktop) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum PrimaryMediaPhase {
    Negotiating,
    DeviceSending,
    ClientPresenting,
    Degraded,
    MediaSourceLost,
    Failed,
}

impl PrimaryMediaPhase {
    pub(in crate::daemon::plugins::remote_desktop) const fn as_str(self) -> &'static str {
        match self {
            Self::Negotiating => "negotiating",
            Self::DeviceSending => "device_sending",
            Self::ClientPresenting => "client_presenting",
            Self::Degraded => "degraded",
            Self::MediaSourceLost => "media_source_lost",
            Self::Failed => "failed",
        }
    }

    const fn device_sending(self) -> bool {
        matches!(
            self,
            Self::DeviceSending | Self::ClientPresenting | Self::Degraded
        )
    }

    const fn client_presenting(self) -> bool {
        matches!(self, Self::ClientPresenting)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrimaryMediaState {
    epoch: TransportEpoch,
    phase: PrimaryMediaPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopMediaStats {
    value: Value,
}

impl RemoteDesktopMediaStats {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn new(value: Value) -> Self {
        Self { value }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        self.value.clone()
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTransportState {
    primary: Option<PrimaryMediaState>,
    media_stats: Option<RemoteDesktopMediaStats>,
    preview_stop_tx: Option<watch::Sender<bool>>,
}

impl RemoteDesktopTransportState {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            primary: None,
            media_stats: None,
            preview_stop_tx: None,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn active_epoch(
        &self,
    ) -> Option<TransportEpoch> {
        self.primary.map(|state| state.epoch)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn accepts_epoch(
        &self,
        epoch: TransportEpoch,
    ) -> bool {
        self.active_epoch() == Some(epoch)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn primary_phase(
        &self,
    ) -> Option<PrimaryMediaPhase> {
        self.primary.map(|state| state.phase)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn begin_primary(
        &mut self,
        epoch: TransportEpoch,
    ) {
        self.primary = Some(PrimaryMediaState {
            epoch,
            phase: PrimaryMediaPhase::Negotiating,
        });
        self.media_stats = None;
    }

    fn transition_primary(&mut self, epoch: TransportEpoch, phase: PrimaryMediaPhase) -> bool {
        let Some(primary) = self.primary.as_mut() else {
            return false;
        };
        if primary.epoch != epoch
            || primary.phase == phase
            || !Self::can_transition_primary(primary.phase, phase)
        {
            return false;
        }
        primary.phase = phase;
        true
    }

    fn can_transition_primary(from: PrimaryMediaPhase, to: PrimaryMediaPhase) -> bool {
        match from {
            PrimaryMediaPhase::Negotiating => {
                matches!(
                    to,
                    PrimaryMediaPhase::DeviceSending | PrimaryMediaPhase::Failed
                )
            }
            PrimaryMediaPhase::DeviceSending => matches!(
                to,
                PrimaryMediaPhase::ClientPresenting
                    | PrimaryMediaPhase::Degraded
                    | PrimaryMediaPhase::MediaSourceLost
                    | PrimaryMediaPhase::Failed
            ),
            PrimaryMediaPhase::ClientPresenting => matches!(
                to,
                PrimaryMediaPhase::Degraded
                    | PrimaryMediaPhase::MediaSourceLost
                    | PrimaryMediaPhase::Failed
            ),
            PrimaryMediaPhase::Degraded => matches!(
                to,
                PrimaryMediaPhase::ClientPresenting
                    | PrimaryMediaPhase::MediaSourceLost
                    | PrimaryMediaPhase::Failed
            ),
            PrimaryMediaPhase::MediaSourceLost => matches!(to, PrimaryMediaPhase::Failed),
            PrimaryMediaPhase::Failed => false,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_device_sending(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        self.transition_primary(epoch, PrimaryMediaPhase::DeviceSending)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_client_presenting(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        let Some(primary) = self.primary else {
            return false;
        };
        if primary.epoch != epoch || !primary.phase.device_sending() {
            return false;
        }
        self.transition_primary(epoch, PrimaryMediaPhase::ClientPresenting)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_client_stalled(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        self.transition_primary(epoch, PrimaryMediaPhase::Degraded)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_failed(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        self.transition_primary(epoch, PrimaryMediaPhase::Failed)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_media_source_lost(
        &mut self,
        epoch: TransportEpoch,
    ) -> bool {
        if self.primary_phase() == Some(PrimaryMediaPhase::Failed) {
            return false;
        }
        self.transition_primary(epoch, PrimaryMediaPhase::MediaSourceLost)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn media_transport_ready(&self) -> bool {
        self.primary
            .is_some_and(|state| state.phase.device_sending())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn client_media_ready(&self) -> bool {
        self.primary
            .is_some_and(|state| state.phase.client_presenting())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn media_stats(&self) -> Option<Value> {
        self.media_stats
            .as_ref()
            .map(RemoteDesktopMediaStats::to_value)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn record_media_stats(
        &mut self,
        epoch: TransportEpoch,
        stats: Value,
    ) -> bool {
        if !self.accepts_epoch(epoch) {
            return false;
        }
        self.media_stats = Some(RemoteDesktopMediaStats::new(stats));
        true
    }

    pub(in crate::daemon::plugins::remote_desktop) fn preview_attached(&self) -> bool {
        self.preview_stop_tx.is_some()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn attach_preview_transport(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) -> Option<watch::Sender<bool>> {
        self.preview_stop_tx.replace(stop_tx)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport(
        &mut self,
    ) -> Option<watch::Sender<bool>> {
        self.preview_stop_tx.take()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn projection(&self) -> Value {
        json!({
            "epoch": self.active_epoch().map(TransportEpoch::value),
            "primary": self.primary_phase().map(PrimaryMediaPhase::as_str).unwrap_or("idle"),
            "device_sending": self.media_transport_ready(),
            "client_presenting": self.client_media_ready(),
            "diagnostic_preview": if self.preview_attached() { "attached" } else { "detached" },
        })
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn install_preview_transport_for_test(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) {
        self.preview_stop_tx = Some(stop_tx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_detach_does_not_clear_primary_media() {
        let mut transport = RemoteDesktopTransportState::new();
        let epoch = TransportEpoch::new(1);
        let (stop_tx, _stop_rx) = watch::channel(false);
        transport.begin_primary(epoch);
        assert!(transport.mark_device_sending(epoch));
        transport.attach_preview_transport(stop_tx);

        assert!(transport.detach_preview_transport().is_some());
        assert!(transport.media_transport_ready());
        assert!(!transport.preview_attached());
    }

    #[test]
    fn stale_transport_epoch_cannot_mutate_current_generation() {
        let mut transport = RemoteDesktopTransportState::new();
        let old = TransportEpoch::new(1);
        let current = TransportEpoch::new(2);
        transport.begin_primary(old);
        transport.begin_primary(current);

        assert!(!transport.mark_device_sending(old));
        assert!(!transport.mark_failed(old));
        assert_eq!(
            transport.primary_phase(),
            Some(PrimaryMediaPhase::Negotiating)
        );
        assert!(transport.mark_device_sending(current));
    }

    #[test]
    fn client_presentation_requires_device_sending() {
        let mut transport = RemoteDesktopTransportState::new();
        let epoch = TransportEpoch::new(7);
        transport.begin_primary(epoch);

        assert!(!transport.mark_client_presenting(epoch));
        assert!(transport.mark_device_sending(epoch));
        assert!(transport.mark_client_presenting(epoch));
        assert!(transport.client_media_ready());
    }

    #[test]
    fn media_source_lost_clears_device_sending_readiness() {
        let mut transport = RemoteDesktopTransportState::new();
        let epoch = TransportEpoch::new(1);
        transport.begin_primary(epoch);
        assert!(transport.mark_device_sending(epoch));
        assert!(transport.media_transport_ready());

        assert!(transport.mark_media_source_lost(epoch));

        assert_eq!(
            transport.primary_phase(),
            Some(PrimaryMediaPhase::MediaSourceLost)
        );
        assert!(!transport.media_transport_ready());
        assert_eq!(
            transport.projection()["primary"],
            json!("media_source_lost")
        );
        assert_eq!(transport.projection()["device_sending"], json!(false));
    }

    #[test]
    fn media_source_lost_does_not_rewrite_terminal_transport_failure() {
        let mut transport = RemoteDesktopTransportState::new();
        let epoch = TransportEpoch::new(2);
        transport.begin_primary(epoch);
        assert!(transport.mark_failed(epoch));

        assert!(!transport.mark_media_source_lost(epoch));

        assert_eq!(transport.primary_phase(), Some(PrimaryMediaPhase::Failed));
    }

    #[test]
    fn media_source_lost_is_absorbing_until_new_epoch_or_failure() {
        let mut transport = RemoteDesktopTransportState::new();
        let epoch = TransportEpoch::new(3);
        transport.begin_primary(epoch);
        assert!(transport.mark_device_sending(epoch));
        assert!(transport.mark_media_source_lost(epoch));

        assert!(!transport.mark_client_stalled(epoch));
        assert!(!transport.mark_device_sending(epoch));
        assert!(!transport.mark_client_presenting(epoch));
        assert_eq!(
            transport.primary_phase(),
            Some(PrimaryMediaPhase::MediaSourceLost)
        );
        assert!(!transport.media_transport_ready());
        assert_eq!(transport.projection()["device_sending"], json!(false));

        assert!(transport.mark_failed(epoch));
        assert_eq!(transport.primary_phase(), Some(PrimaryMediaPhase::Failed));

        let next_epoch = TransportEpoch::new(4);
        transport.begin_primary(next_epoch);
        assert_eq!(
            transport.primary_phase(),
            Some(PrimaryMediaPhase::Negotiating)
        );
        assert!(transport.mark_device_sending(next_epoch));
    }
}
