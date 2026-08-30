// EasyNet CLI — bounded encoded-audio WebRTC writer
// =================================================
//
// Owns the transport-only portion of RemoteApp audio: an independently
// scheduled, hard-bounded freshest-data queue feeding one negotiated WebRTC
// audio track. Capture and codec implementations stay outside this module.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use rtc::media::Sample;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;

pub(super) const ENCODED_AUDIO_QUEUE_DEPTH: usize = 4;

pub(in crate::daemon::plugins::remote_desktop) fn is_webrtc_sender_backpressure(
    error: &impl std::fmt::Display,
) -> bool {
    let message = error.to_string();
    message.contains("SenderRtp") && message.contains("Full(")
}

/// A hard-bounded FIFO that preserves the freshest real-time media.
#[derive(Debug)]
pub(super) struct BoundedPendingWrites<T> {
    values: Mutex<VecDeque<T>>,
    capacity: usize,
}

impl<T> BoundedPendingWrites<T> {
    pub(super) fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "bounded media queue capacity must be positive"
        );
        Self {
            values: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Returns true when an older pending value was dropped.
    pub(super) fn push_fresh(&self, value: T) -> bool {
        let mut values = self
            .values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dropped = if values.len() == self.capacity {
            values.pop_front();
            true
        } else {
            false
        };
        values.push_back(value);
        dropped
    }

    pub(super) fn pop_oldest(&self) -> Option<T> {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
    }

    pub(super) fn len(&self) -> usize {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub(super) fn clear(&self) {
        self.values
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

#[derive(Debug)]
pub(super) struct EncodedAudioPacket {
    pub(super) payload: Bytes,
    pub(super) duration: Duration,
}

#[derive(Debug, Clone)]
pub(super) struct EncodedAudioWriterSnapshot {
    pub(super) packets_written: u64,
    pub(super) bytes_written: u64,
    pub(super) sender_backpressure_errors: u64,
    pub(super) fatal_error: Option<String>,
}

#[derive(Debug, Default)]
struct EncodedAudioWriterState {
    packets_written: AtomicU64,
    bytes_written: AtomicU64,
    sender_backpressure_errors: AtomicU64,
    fatal_error: Mutex<Option<String>>,
}

impl EncodedAudioWriterState {
    fn record_written(&self, bytes_len: u64) {
        self.packets_written.fetch_add(1, Ordering::Relaxed);
        self.bytes_written.fetch_add(bytes_len, Ordering::Relaxed);
    }

    fn record_failure(&self, error: String) -> bool {
        if is_webrtc_sender_backpressure(&error) {
            self.sender_backpressure_errors
                .fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let mut fatal_error = self
            .fatal_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if fatal_error.is_none() {
            *fatal_error = Some(error);
        }
        true
    }

    fn snapshot(&self) -> EncodedAudioWriterSnapshot {
        EncodedAudioWriterSnapshot {
            packets_written: self.packets_written.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            sender_backpressure_errors: self.sender_backpressure_errors.load(Ordering::Relaxed),
            fatal_error: self
                .fatal_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
        }
    }
}

pub(super) struct EncodedAudioWriter {
    pending: Arc<BoundedPendingWrites<EncodedAudioPacket>>,
    pending_notify: Arc<tokio::sync::Notify>,
    state: Arc<EncodedAudioWriterState>,
    task: tokio::task::JoinHandle<()>,
}

impl EncodedAudioWriter {
    pub(super) fn spawn(track: Arc<TrackLocalStaticSample>, ssrc: u32, payload_type: u8) -> Self {
        let pending: Arc<BoundedPendingWrites<EncodedAudioPacket>> =
            Arc::new(BoundedPendingWrites::new(ENCODED_AUDIO_QUEUE_DEPTH));
        let pending_notify = Arc::new(tokio::sync::Notify::new());
        let state = Arc::new(EncodedAudioWriterState::default());
        let worker_pending = Arc::clone(&pending);
        let worker_notify = Arc::clone(&pending_notify);
        let worker_state = Arc::clone(&state);
        let task = tokio::spawn(async move {
            loop {
                worker_notify.notified().await;
                while let Some(packet) = worker_pending.pop_oldest() {
                    let bytes_len = packet.payload.len() as u64;
                    match track
                        .sample_writer(ssrc, payload_type)
                        .write_sample(&Sample {
                            data: packet.payload,
                            duration: packet.duration,
                            ..Default::default()
                        })
                        .await
                    {
                        Ok(()) => worker_state.record_written(bytes_len),
                        Err(error) if worker_state.record_failure(error.to_string()) => return,
                        Err(_) => {}
                    }
                }
            }
        });
        Self {
            pending,
            pending_notify,
            state,
            task,
        }
    }

    pub(super) fn enqueue(&self, packet: EncodedAudioPacket) -> bool {
        let dropped = self.pending.push_fresh(packet);
        self.pending_notify.notify_one();
        dropped
    }

    pub(super) fn queued_packets(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn snapshot(&self) -> EncodedAudioWriterSnapshot {
        self.state.snapshot()
    }

    pub(super) async fn shutdown_discard(mut self) {
        self.pending.clear();
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for EncodedAudioWriter {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_pending_writes_drop_oldest_without_exceeding_capacity() {
        let pending = BoundedPendingWrites::new(3);
        assert!(!pending.push_fresh(1_u64));
        assert!(!pending.push_fresh(2_u64));
        assert!(!pending.push_fresh(3_u64));
        assert!(pending.push_fresh(4_u64));
        assert_eq!(pending.len(), 3);
        assert_eq!(pending.pop_oldest(), Some(2));
        assert_eq!(pending.pop_oldest(), Some(3));
        assert_eq!(pending.pop_oldest(), Some(4));
        assert_eq!(pending.pop_oldest(), None);
    }

    #[test]
    fn writer_state_separates_backpressure_from_terminal_failure() {
        let state = EncodedAudioWriterState::default();
        state.record_written(128);
        assert!(!state.record_failure("SenderRtp Full(1)".to_string()));
        assert!(state.record_failure("transport closed".to_string()));
        assert!(state.record_failure("later fatal error".to_string()));
        assert_eq!(state.snapshot().packets_written, 1);
        assert_eq!(state.snapshot().bytes_written, 128);
        assert_eq!(state.snapshot().sender_backpressure_errors, 1);
        assert_eq!(
            state.snapshot().fatal_error.as_deref(),
            Some("transport closed")
        );
    }
}
