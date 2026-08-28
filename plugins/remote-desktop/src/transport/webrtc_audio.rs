// EasyNet CLI — bounded RemoteApp WebRTC audio transport
// ======================================================
//
// File: plugins/remote-desktop/src/transport/webrtc_audio.rs
// Description: Platform-neutral host-audio encode and WebRTC send pipeline.
//
// Protocol Responsibility:
// - None. The caller supplies an already admitted session audio track and owns
//   authority, transport epochs, cancellation, and terminal lifecycle.
//
// Implementation Approach:
// - Platform capture adapters publish normalized PCM into a bounded freshest-
//   data FIFO. A shared Opus encoder feeds an independently bounded RTP writer.
// - Slow or failed audio transport cannot block video/session control.
//
// Usage Contract:
// - Exactly one pipeline owns each negotiated audio track.
// - `drain` must be called by the platform media loop; terminal paths call
//   `shutdown_discard` and await the sole writer before reporting completion.
//   Drop remains a last-resort abort for unwinding paths.
//
// Architectural Position:
// - RemoteDesktop plugin transport strategy shared by every host-audio adapter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::Track;

use crate::daemon::plugins::remote_desktop::media::audio::{
    AudioCaptureEvent, AudioSink, CapturedAudioChunk, RemoteAppOpusEncoder,
    REMOTEAPP_AUDIO_CHANNELS, REMOTEAPP_AUDIO_CODEC, REMOTEAPP_AUDIO_PAYLOAD_CONTENT_TYPE,
    REMOTEAPP_AUDIO_SAMPLE_RATE_HZ,
};
use crate::daemon::plugins::remote_desktop::transport::webrtc_encoded_audio::{
    BoundedPendingWrites, EncodedAudioPacket, EncodedAudioWriter, EncodedAudioWriterSnapshot,
    ENCODED_AUDIO_QUEUE_DEPTH,
};

pub(super) const AUDIO_CAPTURE_QUEUE_DEPTH: usize = 4;

#[derive(Debug, Clone)]
pub(super) struct RemoteAppAudioStats {
    pub(super) backend_available: bool,
    pub(super) negotiated: bool,
    pub(super) sender_ready: bool,
    pub(super) packets_written: u64,
    pub(super) bytes_written: u64,
    pub(super) capture_chunks_dropped: u64,
    pub(super) queued_packets: usize,
    pub(super) max_queued_packets: usize,
    pub(super) stale_packets_dropped: u64,
    pub(super) sender_backpressure_errors: u64,
    pub(super) sender_backpressure_drops: u64,
    pub(super) capture_source: Option<&'static str>,
    pub(super) capture_chunks_forwarded: u64,
    pub(super) capture_backend_chunks_dropped: u64,
    pub(super) capture_stall_events: u64,
    pub(super) capture_recovery_events: u64,
    pub(super) precommit_chunks_discarded: u64,
    pub(super) blocker: Option<String>,
}

impl RemoteAppAudioStats {
    /// Return the first non-recoverable failure for an accepted audio track.
    /// A non-negotiated track may carry a diagnostic blocker, but it is not a
    /// failure of the active media contract.
    pub(super) fn terminal_failure(&self) -> Option<&str> {
        self.negotiated.then_some(())?;
        self.blocker.as_deref()
    }

    pub(super) fn append_json(&self, payload: &mut Map<String, Value>) {
        let capture_started = self.capture_source.is_some();
        let operational_ready = self.negotiated
            && self.backend_available
            && self.sender_ready
            && capture_started
            && self.blocker.is_none();
        payload.insert(
            "audio_codec".to_string(),
            self.negotiated
                .then(|| json!(REMOTEAPP_AUDIO_CODEC))
                .unwrap_or(Value::Null),
        );
        // `audio_ready` remains the public compatibility projection. The
        // explicit fields prevent callers from conflating an operational
        // capture/sender with observed packets or client-side decode.
        payload.insert("audio_ready".to_string(), json!(operational_ready));
        payload.insert(
            "audio_operational_ready".to_string(),
            json!(operational_ready),
        );
        payload.insert("audio_capture_started".to_string(), json!(capture_started));
        payload.insert("audio_sender_ready".to_string(), json!(self.sender_ready));
        payload.insert(
            "audio_media_observed".to_string(),
            json!(self.packets_written > 0),
        );
        payload.insert(
            "host_audio_not_implemented".to_string(),
            json!(!self.backend_available),
        );
        payload.insert(
            "audio_backend_available".to_string(),
            json!(self.backend_available),
        );
        payload.insert(
            "audio_blocker".to_string(),
            self.blocker
                .as_ref()
                .map(|reason| Value::String(reason.clone()))
                .unwrap_or(Value::Null),
        );
        payload.insert(
            "audio_payload_content_type".to_string(),
            self.negotiated
                .then(|| json!(REMOTEAPP_AUDIO_PAYLOAD_CONTENT_TYPE))
                .unwrap_or(Value::Null),
        );
        payload.insert(
            "audio_sample_rate_hz".to_string(),
            json!(self
                .negotiated
                .then_some(REMOTEAPP_AUDIO_SAMPLE_RATE_HZ)
                .unwrap_or(0)),
        );
        payload.insert(
            "audio_channels".to_string(),
            json!(self
                .negotiated
                .then_some(REMOTEAPP_AUDIO_CHANNELS)
                .unwrap_or(0)),
        );
        payload.insert(
            "audio_packets_written".to_string(),
            json!(self.packets_written),
        );
        payload.insert("audio_bytes_written".to_string(), json!(self.bytes_written));
        payload.insert(
            "audio_capture_chunks_dropped".to_string(),
            json!(self.capture_chunks_dropped),
        );
        payload.insert("audio_queue_depth".to_string(), json!(self.queued_packets));
        payload.insert(
            "audio_max_queue_depth".to_string(),
            json!(self.max_queued_packets),
        );
        payload.insert(
            "audio_transport_write_isolated".to_string(),
            json!(self.negotiated),
        );
        payload.insert(
            "audio_drop_stale_packets".to_string(),
            json!(self.negotiated),
        );
        payload.insert(
            "audio_drop_policy".to_string(),
            self.negotiated
                .then(|| json!("bounded_queue_drop_oldest_audio_packet"))
                .unwrap_or(Value::Null),
        );
        payload.insert(
            "audio_stale_packets_dropped".to_string(),
            json!(self.stale_packets_dropped),
        );
        payload.insert(
            "audio_sender_backpressure_errors".to_string(),
            json!(self.sender_backpressure_errors),
        );
        payload.insert(
            "audio_sender_backpressure_drops".to_string(),
            json!(self.sender_backpressure_drops),
        );
        payload.insert(
            "audio_capture_source".to_string(),
            self.capture_source
                .map(|source| Value::String(source.to_string()))
                .unwrap_or(Value::Null),
        );
        payload.insert(
            "audio_capture_chunks_forwarded".to_string(),
            json!(self.capture_chunks_forwarded),
        );
        payload.insert(
            "audio_capture_backend_chunks_dropped".to_string(),
            json!(self.capture_backend_chunks_dropped),
        );
        payload.insert(
            "audio_capture_stall_events".to_string(),
            json!(self.capture_stall_events),
        );
        payload.insert(
            "audio_capture_recovery_events".to_string(),
            json!(self.capture_recovery_events),
        );
        payload.insert(
            "audio_precommit_chunks_discarded".to_string(),
            json!(self.precommit_chunks_discarded),
        );
    }
}

pub(super) struct RemoteAppAudioPipeline {
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    payload_type: u8,
    encoder: Option<RemoteAppOpusEncoder>,
    capture_pending: Arc<BoundedPendingWrites<CapturedAudioChunk>>,
    capture_error: Arc<Mutex<Option<String>>>,
    capture_chunks_dropped: Arc<AtomicU64>,
    writer: Option<EncodedAudioWriter>,
    stale_packets_dropped: u64,
    blocker: Option<String>,
    capture_source: Option<&'static str>,
}

impl RemoteAppAudioPipeline {
    pub(super) async fn new(
        track: &Arc<TrackLocalStaticSample>,
        payload_type: u8,
    ) -> anyhow::Result<(AudioSink, Self)> {
        let capture_pending = Arc::new(BoundedPendingWrites::new(AUDIO_CAPTURE_QUEUE_DEPTH));
        let capture_error = Arc::new(Mutex::new(None));
        let capture_chunks_dropped = Arc::new(AtomicU64::new(0));
        let pending_for_sink = Arc::clone(&capture_pending);
        let error_for_sink = Arc::clone(&capture_error);
        let dropped_for_sink = Arc::clone(&capture_chunks_dropped);
        let sink: AudioSink = Arc::new(move |event: AudioCaptureEvent| match event {
            Ok(chunk) => {
                if pending_for_sink.push_fresh(chunk) {
                    dropped_for_sink.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(reason) => {
                *error_for_sink
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reason);
            }
        });
        let ssrc = track
            .ssrcs()
            .await
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("direct WebRTC audio track has no SSRC"))?;
        let writer = EncodedAudioWriter::spawn(Arc::clone(track), ssrc, payload_type);
        Ok((
            sink,
            Self {
                track: Arc::clone(track),
                ssrc,
                payload_type,
                encoder: Some(RemoteAppOpusEncoder::new()?),
                capture_pending,
                capture_error,
                capture_chunks_dropped,
                writer: Some(writer),
                stale_packets_dropped: 0,
                blocker: None,
                capture_source: None,
            },
        ))
    }

    pub(super) fn drain(&mut self) {
        self.observe_writer_failure();
        let capture_error = self
            .capture_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(reason) = capture_error {
            self.blocker = Some(format!("host_audio_capture_failed: {reason}"));
            self.encoder = None;
            if let Some(writer) = self.writer.as_ref() {
                writer.abort();
            }
        }
        while let Some(chunk) = self.capture_pending.pop_oldest() {
            let Some(encoder) = self.encoder.as_mut() else {
                continue;
            };
            let packets = match encoder.push_chunk(chunk) {
                Ok(packets) => packets,
                Err(error) => {
                    self.blocker = Some(format!("host_audio_encode_failed: {error}"));
                    self.encoder = None;
                    continue;
                }
            };
            for packet in packets {
                let Some(writer) = self.writer.as_ref() else {
                    continue;
                };
                if writer.enqueue(EncodedAudioPacket {
                    payload: packet.payload.into(),
                    duration: packet.duration,
                }) {
                    self.stale_packets_dropped = self.stale_packets_dropped.saturating_add(1);
                }
            }
        }
        self.observe_writer_failure();
    }

    fn observe_writer_failure(&mut self) {
        let Some(writer) = self.writer.as_ref() else {
            return;
        };
        if let Some(error) = writer.snapshot().fatal_error {
            self.blocker = Some(format!("host_audio_send_failed: {error}"));
            self.encoder = None;
            writer.abort();
        }
    }

    /// Establish a hard transport barrier before a media-source rebind.
    ///
    /// The only writer for this WebRTC track is aborted and awaited before the
    /// session can commit another media-source epoch. Capture and encoded
    /// queues are discarded, so no old-target packet can cross the commit.
    pub(super) async fn quiesce_for_rebind(&mut self) -> anyhow::Result<()> {
        let replacement_encoder = RemoteAppOpusEncoder::new()?;
        self.capture_pending.clear();
        *self
            .capture_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        if let Some(writer) = self.writer.take() {
            writer.shutdown_discard().await;
        }
        self.encoder = Some(replacement_encoder);
        self.capture_chunks_dropped.store(0, Ordering::Relaxed);
        self.stale_packets_dropped = 0;
        self.blocker = None;
        Ok(())
    }

    /// Start the sole writer for the newly active (or rolled-back) source.
    /// All fallible codec/track discovery work is completed before quiesce, so
    /// activation after the canonical session commit is intentionally
    /// infallible.
    pub(super) fn activate_after_rebind(&mut self) {
        assert!(
            self.writer.is_none(),
            "audio writer must be quiesced before media-source activation"
        );
        self.writer = Some(EncodedAudioWriter::spawn(
            Arc::clone(&self.track),
            self.ssrc,
            self.payload_type,
        ));
    }

    /// Stop the transport writer and discard all queued real-time media before
    /// the owning session task reports terminal completion.
    pub(super) async fn shutdown_discard(&mut self) {
        self.capture_pending.clear();
        *self
            .capture_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.encoder = None;
        if let Some(writer) = self.writer.take() {
            writer.shutdown_discard().await;
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn set_capture_source(&mut self, source: &'static str) {
        self.capture_source = Some(source);
    }

    pub(super) fn stats(&self) -> RemoteAppAudioStats {
        let writer = self
            .writer
            .as_ref()
            .map(EncodedAudioWriter::snapshot)
            .unwrap_or(EncodedAudioWriterSnapshot {
                packets_written: 0,
                bytes_written: 0,
                sender_backpressure_errors: 0,
                fatal_error: None,
            });
        RemoteAppAudioStats {
            backend_available: true,
            negotiated: true,
            sender_ready: self.writer.is_some(),
            packets_written: writer.packets_written,
            bytes_written: writer.bytes_written,
            capture_chunks_dropped: self.capture_chunks_dropped.load(Ordering::Relaxed),
            queued_packets: self
                .writer
                .as_ref()
                .map(EncodedAudioWriter::queued_packets)
                .unwrap_or(0),
            max_queued_packets: ENCODED_AUDIO_QUEUE_DEPTH,
            stale_packets_dropped: self.stale_packets_dropped,
            sender_backpressure_errors: writer.sender_backpressure_errors,
            sender_backpressure_drops: self
                .stale_packets_dropped
                .saturating_add(writer.sender_backpressure_errors),
            capture_source: self.capture_source,
            capture_chunks_forwarded: 0,
            capture_backend_chunks_dropped: 0,
            capture_stall_events: 0,
            capture_recovery_events: 0,
            precommit_chunks_discarded: 0,
            blocker: self.blocker.clone().or_else(|| {
                writer
                    .fatal_error
                    .map(|error| format!("host_audio_send_failed: {error}"))
            }),
        }
    }
}

pub(super) fn audio_stats_not_negotiated() -> RemoteAppAudioStats {
    RemoteAppAudioStats {
        backend_available: true,
        negotiated: false,
        sender_ready: false,
        packets_written: 0,
        bytes_written: 0,
        capture_chunks_dropped: 0,
        queued_packets: 0,
        max_queued_packets: 0,
        stale_packets_dropped: 0,
        sender_backpressure_errors: 0,
        sender_backpressure_drops: 0,
        capture_source: None,
        capture_chunks_forwarded: 0,
        capture_backend_chunks_dropped: 0,
        capture_stall_events: 0,
        capture_recovery_events: 0,
        precommit_chunks_discarded: 0,
        blocker: Some("host_audio_not_negotiated".to_string()),
    }
}

pub(super) fn audio_stats_backend_unavailable(reason: String) -> RemoteAppAudioStats {
    RemoteAppAudioStats {
        backend_available: false,
        negotiated: true,
        sender_ready: false,
        packets_written: 0,
        bytes_written: 0,
        capture_chunks_dropped: 0,
        queued_packets: 0,
        max_queued_packets: 0,
        stale_packets_dropped: 0,
        sender_backpressure_errors: 0,
        sender_backpressure_drops: 0,
        capture_source: None,
        capture_chunks_forwarded: 0,
        capture_backend_chunks_dropped: 0,
        capture_stall_events: 0,
        capture_recovery_events: 0,
        precommit_chunks_discarded: 0,
        blocker: Some(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiated_audio_blocker_is_terminal_but_not_negotiated_is_not() {
        let not_negotiated = audio_stats_not_negotiated();
        assert_eq!(not_negotiated.terminal_failure(), None);

        let mut negotiated = not_negotiated.clone();
        negotiated.negotiated = true;
        negotiated.backend_available = false;
        negotiated.blocker = Some("host_audio_capture_failed: device lost".to_string());
        assert_eq!(
            negotiated.terminal_failure(),
            Some("host_audio_capture_failed: device lost")
        );

        negotiated.blocker = None;
        assert_eq!(negotiated.terminal_failure(), None);
    }
}
