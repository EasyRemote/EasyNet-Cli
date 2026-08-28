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

use std::time::Instant;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::daemon::plugins::remote_desktop) struct PreviewTransportEpoch(u64);

impl PreviewTransportEpoch {
    pub(in crate::daemon::plugins::remote_desktop) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone)]
struct PreviewTransportState {
    epoch: PreviewTransportEpoch,
    stop_tx: watch::Sender<bool>,
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

/// Latest authenticated receiver-side media feedback for one transport epoch.
///
/// Cumulative counters are kept typed instead of being re-read from the public
/// JSON projection by the encoder loop. A newer browser sample may influence
/// quality adaptation, but it cannot change session authority or lifecycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::daemon::plugins::remote_desktop) struct ClientMediaFeedback {
    /// Monotonic admission order owned by the active transport epoch. This is
    /// the only ordering key consumed by encoder adaptation.
    pub(in crate::daemon::plugins::remote_desktop) admission_sequence: u64,
    /// Process-monotonic admission instant used for freshness decisions.
    pub(in crate::daemon::plugins::remote_desktop) received_at: Instant,
    /// Daemon-local wall-clock admission time retained for audit projection;
    /// encoder freshness uses `received_at`, never this or browser wall time.
    pub(in crate::daemon::plugins::remote_desktop) received_at_ms: u64,
    pub(in crate::daemon::plugins::remote_desktop) sampled_at_ms: u64,
    pub(in crate::daemon::plugins::remote_desktop) frames_dropped: u64,
    pub(in crate::daemon::plugins::remote_desktop) freeze_count: u64,
    pub(in crate::daemon::plugins::remote_desktop) jitter_buffer_avg_ms: f64,
    pub(in crate::daemon::plugins::remote_desktop) jitter_buffer_target_avg_ms: f64,
}

/// Authenticated client receiver proof admitted for one exact transport and
/// target-binding generation. Browser timestamps remain audit-only; product
/// freshness is derived from the daemon-owned monotonic `received_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct ClientRenderEvidence {
    pub(in crate::daemon::plugins::remote_desktop) admission_sequence: u64,
    pub(in crate::daemon::plugins::remote_desktop) received_at: Instant,
    pub(in crate::daemon::plugins::remote_desktop) received_at_ms: u64,
    pub(in crate::daemon::plugins::remote_desktop) session_id: String,
    pub(in crate::daemon::plugins::remote_desktop) selected_resource_ura: String,
    pub(in crate::daemon::plugins::remote_desktop) transport_epoch: u64,
    pub(in crate::daemon::plugins::remote_desktop) binding_id: String,
    pub(in crate::daemon::plugins::remote_desktop) binding_epoch: u64,
    pub(in crate::daemon::plugins::remote_desktop) media_source_epoch: u64,
    pub(in crate::daemon::plugins::remote_desktop) media_pipeline_id: String,
    pub(in crate::daemon::plugins::remote_desktop) video_codec: String,
    pub(in crate::daemon::plugins::remote_desktop) video_transport: String,
    pub(in crate::daemon::plugins::remote_desktop) audio_codec: Option<String>,
    pub(in crate::daemon::plugins::remote_desktop) decoded_video_frames: u64,
    pub(in crate::daemon::plugins::remote_desktop) decoded_audio_packets: u64,
    pub(in crate::daemon::plugins::remote_desktop) decoded_audio_samples: u64,
    pub(in crate::daemon::plugins::remote_desktop) frame_width: u64,
    pub(in crate::daemon::plugins::remote_desktop) frame_height: u64,
}

impl ClientRenderEvidence {
    fn from_stats_patch(
        stats: &Value,
        admission_sequence: u64,
        received_at: Instant,
        received_at_ms: u64,
    ) -> Option<Self> {
        let probe = stats.get("render_probe")?.as_object()?;
        (probe.get("evidence_authority")?.as_str()? == "client_reported").then_some(())?;
        (probe.get("probe_source")?.as_str()? == "browser_webrtc_receiver").then_some(())?;
        Some(Self {
            admission_sequence,
            received_at,
            received_at_ms,
            session_id: probe.get("session_id")?.as_str()?.to_string(),
            selected_resource_ura: probe.get("selected_resource_ura")?.as_str()?.to_string(),
            transport_epoch: probe.get("transport_epoch")?.as_u64()?,
            binding_id: probe.get("binding_id")?.as_str()?.to_string(),
            binding_epoch: probe.get("binding_epoch")?.as_u64()?,
            media_source_epoch: probe.get("media_source_epoch")?.as_u64()?,
            media_pipeline_id: probe.get("media_pipeline_id")?.as_str()?.to_string(),
            video_codec: probe.get("video_codec")?.as_str()?.to_string(),
            video_transport: probe.get("video_transport")?.as_str()?.to_string(),
            audio_codec: probe
                .get("audio_codec")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            decoded_video_frames: probe.get("decoded_video_frames")?.as_u64()?,
            decoded_audio_packets: probe
                .get("decoded_audio_packets")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            decoded_audio_samples: probe
                .get("decoded_audio_samples")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            frame_width: probe.get("frame_width")?.as_u64()?,
            frame_height: probe.get("frame_height")?.as_u64()?,
        })
    }
}

impl ClientMediaFeedback {
    fn from_stats_patch(
        stats: &Value,
        admission_sequence: u64,
        received_at: Instant,
        received_at_ms: u64,
    ) -> Option<Self> {
        let browser = stats.get("browser_stats")?.as_object()?;
        Some(Self {
            admission_sequence,
            received_at,
            received_at_ms,
            sampled_at_ms: browser.get("sampled_at_ms")?.as_u64()?,
            frames_dropped: browser
                .get("frames_dropped")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            freeze_count: browser
                .get("freeze_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            jitter_buffer_avg_ms: browser
                .get("jitter_buffer_avg_ms")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(0.0),
            jitter_buffer_target_avg_ms: browser
                .get("jitter_buffer_target_avg_ms")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(0.0),
        })
    }
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
    epoch_high_watermark: u64,
    media_stats: Option<RemoteDesktopMediaStats>,
    client_media_feedback: Option<ClientMediaFeedback>,
    client_media_feedback_sequence: u64,
    client_render_evidence: Option<ClientRenderEvidence>,
    client_render_evidence_sequence: u64,
    preview: Option<PreviewTransportState>,
    preview_epoch_high_watermark: u64,
}

impl RemoteDesktopTransportState {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            primary: None,
            epoch_high_watermark: 0,
            media_stats: None,
            client_media_feedback: None,
            client_media_feedback_sequence: 0,
            client_render_evidence: None,
            client_render_evidence_sequence: 0,
            preview: None,
            preview_epoch_high_watermark: 0,
        }
    }

    /// Rehydrate session-scoped epoch history without pretending that a
    /// process-local endpoint survived daemon restart.
    pub(in crate::daemon::plugins::remote_desktop) fn rehydrate(epoch_high_watermark: u64) -> Self {
        Self {
            primary: None,
            epoch_high_watermark,
            media_stats: None,
            client_media_feedback: None,
            client_media_feedback_sequence: 0,
            client_render_evidence: None,
            client_render_evidence_sequence: 0,
            preview: None,
            preview_epoch_high_watermark: 0,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn epoch_high_watermark(&self) -> u64 {
        self.epoch_high_watermark
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
    ) -> bool {
        if epoch.value() <= self.epoch_high_watermark {
            return false;
        }
        self.primary = Some(PrimaryMediaState {
            epoch,
            phase: PrimaryMediaPhase::Negotiating,
        });
        self.epoch_high_watermark = epoch.value();
        self.media_stats = None;
        self.client_media_feedback = None;
        self.client_media_feedback_sequence = 0;
        self.client_render_evidence = None;
        self.client_render_evidence_sequence = 0;
        true
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

    pub(in crate::daemon::plugins::remote_desktop) fn merge_media_stats(
        &mut self,
        epoch: TransportEpoch,
        stats: Value,
    ) -> bool {
        if !self.accepts_epoch(epoch) {
            return false;
        }
        let received_at = Instant::now();
        let received_at_ms = crate::daemon::plugins::remote_desktop::session::now_ms();
        let next_feedback_sequence = self.client_media_feedback_sequence.saturating_add(1);
        if let Some(feedback) = ClientMediaFeedback::from_stats_patch(
            &stats,
            next_feedback_sequence,
            received_at,
            received_at_ms,
        ) {
            // Arrival is already authenticated and epoch-fenced. Browser wall
            // clocks are not comparable to the daemon clock and therefore
            // cannot own ordering or poison the feedback high-water mark.
            self.client_media_feedback_sequence = next_feedback_sequence;
            self.client_media_feedback = Some(feedback);
        }
        let next_render_sequence = self.client_render_evidence_sequence.saturating_add(1);
        if let Some(evidence) = ClientRenderEvidence::from_stats_patch(
            &stats,
            next_render_sequence,
            received_at,
            received_at_ms,
        ) {
            self.client_render_evidence_sequence = next_render_sequence;
            self.client_render_evidence = Some(evidence);
        }
        let mut merged = self.media_stats().unwrap_or_else(|| json!({}));
        merge_json_object(&mut merged, stats);
        self.media_stats = Some(RemoteDesktopMediaStats::new(merged));
        true
    }

    pub(in crate::daemon::plugins::remote_desktop) fn client_media_feedback(
        &self,
        epoch: TransportEpoch,
    ) -> Option<ClientMediaFeedback> {
        self.accepts_epoch(epoch)
            .then_some(self.client_media_feedback)
            .flatten()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn client_render_evidence(
        &self,
        epoch: TransportEpoch,
    ) -> Option<ClientRenderEvidence> {
        self.accepts_epoch(epoch)
            .then(|| self.client_render_evidence.clone())
            .flatten()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn preview_attached(&self) -> bool {
        self.preview.is_some()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn preview_epoch(
        &self,
    ) -> Option<PreviewTransportEpoch> {
        self.preview.as_ref().map(|preview| preview.epoch)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn attach_preview_transport(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) -> (PreviewTransportEpoch, Option<watch::Sender<bool>>) {
        self.preview_epoch_high_watermark = self.preview_epoch_high_watermark.saturating_add(1);
        let epoch = PreviewTransportEpoch::new(self.preview_epoch_high_watermark);
        let old = self
            .preview
            .replace(PreviewTransportState { epoch, stop_tx })
            .map(|preview| preview.stop_tx);
        (epoch, old)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport(
        &mut self,
    ) -> Option<watch::Sender<bool>> {
        self.preview.take().map(|preview| preview.stop_tx)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn detach_preview_transport_if_epoch(
        &mut self,
        epoch: PreviewTransportEpoch,
    ) -> Option<watch::Sender<bool>> {
        if self.preview.as_ref().map(|preview| preview.epoch) != Some(epoch) {
            return None;
        }
        self.detach_preview_transport()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn projection(&self) -> Value {
        json!({
            "epoch": self.active_epoch().map(TransportEpoch::value),
            "primary": self.primary_phase().map(PrimaryMediaPhase::as_str).unwrap_or("idle"),
            "device_sending": self.media_transport_ready(),
            "client_presenting": self.client_media_ready(),
            "diagnostic_preview": if self.preview_attached() { "attached" } else { "detached" },
            "diagnostic_preview_epoch": self.preview.as_ref().map(|preview| preview.epoch.value()),
        })
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn install_preview_transport_for_test(
        &mut self,
        stop_tx: watch::Sender<bool>,
    ) -> PreviewTransportEpoch {
        self.attach_preview_transport(stop_tx).0
    }
}

fn merge_json_object(target: &mut Value, patch: Value) {
    let Value::Object(target_object) = target else {
        *target = patch;
        return;
    };
    let Value::Object(patch_object) = patch else {
        return;
    };
    for (key, value) in patch_object {
        match (target_object.get_mut(&key), value) {
            (Some(existing @ Value::Object(_)), patch @ Value::Object(_)) => {
                merge_json_object(existing, patch);
            }
            (_, value) => {
                target_object.insert(key, value);
            }
        }
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
        let (preview_epoch, _) = transport.attach_preview_transport(stop_tx);

        assert!(transport
            .detach_preview_transport_if_epoch(preview_epoch)
            .is_some());
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
    fn receiver_feedback_is_epoch_fenced_and_daemon_admission_ordered() {
        let mut transport = RemoteDesktopTransportState::new();
        let epoch = TransportEpoch::new(7);
        transport.begin_primary(epoch);
        assert!(transport.merge_media_stats(
            epoch,
            json!({
                "browser_stats": {
                    "sampled_at_ms": 2000,
                    "frames_dropped": 4,
                    "freeze_count": 2,
                    "jitter_buffer_avg_ms": 240.0,
                    "jitter_buffer_target_avg_ms": 125.0
                }
            }),
        ));
        let admitted = transport
            .client_media_feedback(epoch)
            .expect("feedback admitted for active epoch");
        assert_eq!(admitted.admission_sequence, 1);
        assert!(admitted.received_at_ms > 0);
        assert_eq!(admitted.sampled_at_ms, 2000);
        assert_eq!(admitted.frames_dropped, 4);
        assert_eq!(admitted.freeze_count, 2);

        assert!(transport.merge_media_stats(
            epoch,
            json!({
                "browser_stats": {
                    "sampled_at_ms": 1000,
                    "frames_dropped": 99,
                    "freeze_count": 99,
                    "jitter_buffer_avg_ms": 999.0,
                    "jitter_buffer_target_avg_ms": 999.0
                }
            }),
        ));
        let latest = transport.client_media_feedback(epoch).unwrap();
        assert_eq!(latest.admission_sequence, 2);
        assert_eq!(
            latest.frames_dropped, 99,
            "daemon admission order, not a remote wall clock, owns latest feedback"
        );
        assert_eq!(
            transport.client_media_feedback(TransportEpoch::new(8)),
            None
        );

        let next_epoch = TransportEpoch::new(8);
        assert!(transport.begin_primary(next_epoch));
        assert_eq!(transport.client_media_feedback(next_epoch), None);
        assert_eq!(transport.client_media_feedback_sequence, 0);
        assert!(!transport.merge_media_stats(
            epoch,
            json!({
                "browser_stats": {
                    "sampled_at_ms": u64::MAX,
                    "frames_dropped": u64::MAX
                }
            }),
        ));
        assert_eq!(transport.client_media_feedback(next_epoch), None);
    }

    #[test]
    fn render_evidence_is_typed_daemon_ordered_and_cleared_by_new_epoch() {
        let mut transport = RemoteDesktopTransportState::new();
        let epoch = TransportEpoch::new(7);
        assert!(transport.begin_primary(epoch));
        assert!(transport.merge_media_stats(
            epoch,
            json!({
                "render_probe": {
                    "evidence_authority": "client_reported",
                    "probe_source": "browser_webrtc_receiver",
                    "session_id": "rd-evidence",
                    "selected_resource_ura": "easynet:///r/acme/resource/window.evidence",
                    "transport_epoch": 7,
                    "binding_id": "binding-7",
                    "binding_epoch": 3,
                    "media_source_epoch": 9,
                    "media_pipeline_id": "pipeline-7",
                    "video_codec": "h264",
                    "video_transport": "webrtc",
                    "audio_codec": "opus",
                    "decoded_video_frames": 12,
                    "decoded_audio_packets": 4,
                    "decoded_audio_samples": 3_840,
                    "frame_width": 1280,
                    "frame_height": 720
                }
            }),
        ));
        let admitted = transport
            .client_render_evidence(epoch)
            .expect("typed render evidence admitted");
        assert_eq!(admitted.admission_sequence, 1);
        assert!(admitted.received_at_ms > 0);
        assert_eq!(admitted.transport_epoch, 7);
        assert_eq!(admitted.binding_id, "binding-7");
        assert_eq!(admitted.decoded_video_frames, 12);
        assert_eq!(admitted.decoded_audio_samples, 3_840);

        let next_epoch = TransportEpoch::new(8);
        assert!(transport.begin_primary(next_epoch));
        assert_eq!(transport.client_render_evidence(next_epoch), None);
        assert_eq!(transport.client_render_evidence_sequence, 0);
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
