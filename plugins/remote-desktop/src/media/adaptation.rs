// EasyNet CLI — shared remote desktop media adaptation
// ====================================================
//
// File: plugins/remote-desktop/src/media/adaptation.rs
// Description: Platform-neutral receiver-pressure and video-rate policy.
//
// Protocol Responsibility:
// - None. Browser feedback is already admitted and epoch-bound by the session.
//
// Implementation Approach:
// - Convert fresh monotonic receiver counters into bounded pressure deltas.
// - Propose bitrate changes without committing them until an encoder accepts.
// - Derive an interactive FPS ceiling from the applied bitrate.
//
// Usage Contract:
// - Media strategies must call `commit_applied` only after encoder mutation or
//   replacement succeeds. Stale feedback must never influence a new epoch.
//
// Architectural Position:
// - RemoteDesktop plugin media-policy layer, shared by native and baseline
//   capture/encoder strategies.

use std::time::{Duration, Instant};

use crate::daemon::plugins::remote_desktop::session_transport_state::ClientMediaFeedback;

const BITRATE_STEP_KBPS: u32 = 500;
/// The encoder may descend below the public requested-quality floor when the
/// authenticated transport estimate cannot carry that quality. Keeping a
/// decodable low-rate stream is preferable to saturating the sender until the
/// one-frame queue drops every dependency chain.
const ADAPTIVE_MIN_BITRATE_KBPS: u32 = 128;
const RECEIVER_JITTER_PRESSURE_MS: f64 = 100.0;
const RECEIVER_FEEDBACK_MAX_AGE: Duration = Duration::from_secs(10);
const RECEIVER_PRESSURE_RECOVERY_HOLD_SAMPLES: u8 = 8;
const RTP_WRITER_HEADROOM_PERCENT: f64 = 125.0;
const INTERACTIVE_SERVICE_MIN_FPS: u32 = 5;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(in crate::daemon::plugins::remote_desktop) struct ReceiverPressure {
    pub(in crate::daemon::plugins::remote_desktop) frames_dropped_delta: u64,
    pub(in crate::daemon::plugins::remote_desktop) freeze_delta: u64,
    pub(in crate::daemon::plugins::remote_desktop) elevated_jitter: bool,
}

impl ReceiverPressure {
    pub(in crate::daemon::plugins::remote_desktop) fn pressure_units(self) -> u64 {
        self.frames_dropped_delta
            .saturating_add(self.freeze_delta)
            .saturating_add(u64::from(self.elevated_jitter))
    }
}

#[derive(Debug, Default)]
pub(in crate::daemon::plugins::remote_desktop) struct ReceiverPressureTracker {
    last_admission_sequence: u64,
    last_frames_dropped: u64,
    last_freeze_count: u64,
}

impl ReceiverPressureTracker {
    pub(in crate::daemon::plugins::remote_desktop) fn observe(
        &mut self,
        feedback: Option<ClientMediaFeedback>,
        observed_at: Instant,
    ) -> ReceiverPressure {
        let Some(feedback) = feedback else {
            return ReceiverPressure::default();
        };
        if observed_at.saturating_duration_since(feedback.received_at) > RECEIVER_FEEDBACK_MAX_AGE {
            return ReceiverPressure::default();
        }
        if feedback.admission_sequence <= self.last_admission_sequence {
            return ReceiverPressure::default();
        }
        let first_sample = self.last_admission_sequence == 0;
        let pressure = ReceiverPressure {
            frames_dropped_delta: if first_sample {
                0
            } else {
                feedback
                    .frames_dropped
                    .saturating_sub(self.last_frames_dropped)
            },
            freeze_delta: if first_sample {
                0
            } else {
                feedback.freeze_count.saturating_sub(self.last_freeze_count)
            },
            elevated_jitter: !first_sample
                && feedback
                    .jitter_buffer_avg_ms
                    .max(feedback.jitter_buffer_target_avg_ms)
                    >= RECEIVER_JITTER_PRESSURE_MS,
        };
        self.last_admission_sequence = feedback.admission_sequence;
        self.last_frames_dropped = self.last_frames_dropped.max(feedback.frames_dropped);
        self.last_freeze_count = self.last_freeze_count.max(feedback.freeze_count);
        pressure
    }
}

/// Feedback controller shared by every production video encoder.
///
/// A proposal is intentionally separate from commit so telemetry can never
/// claim a bitrate that the active encoder rejected.
#[derive(Debug)]
pub(in crate::daemon::plugins::remote_desktop) struct AdaptiveBitrateController {
    pub(in crate::daemon::plugins::remote_desktop) target_kbps: u32,
    pub(in crate::daemon::plugins::remote_desktop) current_kbps: u32,
    pub(in crate::daemon::plugins::remote_desktop) min_kbps: u32,
    last_input_dropped: u64,
    last_output_dropped: u64,
    recovery_hold_samples: u8,
}

impl AdaptiveBitrateController {
    pub(in crate::daemon::plugins::remote_desktop) fn new(target_kbps: u32) -> Self {
        let target_kbps = target_kbps.max(ADAPTIVE_MIN_BITRATE_KBPS);
        Self {
            target_kbps,
            current_kbps: target_kbps,
            min_kbps: ADAPTIVE_MIN_BITRATE_KBPS.min(target_kbps),
            last_input_dropped: 0,
            last_output_dropped: 0,
            recovery_hold_samples: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::daemon::plugins::remote_desktop) fn propose(
        &mut self,
        input_dropped: u64,
        output_dropped: u64,
        queued_units: usize,
        in_flight_frames: usize,
        receiver_pressure_units: u64,
    ) -> Option<u32> {
        let input_delta = input_dropped.saturating_sub(self.last_input_dropped);
        let output_delta = output_dropped.saturating_sub(self.last_output_dropped);
        self.last_input_dropped = input_dropped;
        self.last_output_dropped = output_dropped;

        let dropped_delta = input_delta
            .saturating_add(output_delta)
            .saturating_add(receiver_pressure_units);
        if dropped_delta > 0 {
            self.recovery_hold_samples = RECEIVER_PRESSURE_RECOVERY_HOLD_SAMPLES;
        } else if self.recovery_hold_samples > 0 {
            self.recovery_hold_samples -= 1;
        }
        let next = adaptive_bitrate_kbps(
            self.current_kbps,
            self.target_kbps,
            self.min_kbps,
            dropped_delta,
            queued_units,
            in_flight_frames,
            self.recovery_hold_samples == 0,
        );
        if next == self.current_kbps {
            return None;
        }
        if next > self.current_kbps && next.abs_diff(self.current_kbps) < BITRATE_STEP_KBPS {
            return Some(
                self.current_kbps
                    .saturating_add(BITRATE_STEP_KBPS)
                    .min(self.target_kbps),
            );
        }
        Some(next)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn commit_applied(&mut self, bitrate_kbps: u32) {
        self.current_kbps = bitrate_kbps.clamp(self.min_kbps, self.target_kbps);
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn effective_fps_for_bitrate(
    requested_fps: u32,
    current_kbps: u32,
    target_kbps: u32,
) -> u32 {
    let requested_fps = requested_fps.max(1);
    let minimum_fps = requested_fps.min(15);
    let target_kbps = target_kbps.max(1);
    requested_fps
        .saturating_mul(current_kbps)
        .saturating_div(target_kbps)
        .clamp(minimum_fps, requested_fps)
}

/// Derive the maximum production rate that the measured RTP writer can drain.
///
/// The 25% headroom prevents a capture/encode producer from running exactly at
/// the p95 service limit. No sample means no service-time restriction; this is
/// distinct from guessing that an absent measurement is zero latency.
pub(in crate::daemon::plugins::remote_desktop) fn effective_fps_for_writer_service(
    requested_fps: u32,
    p95_ms: f64,
    samples: usize,
) -> u32 {
    let requested_fps = requested_fps.max(1);
    if samples == 0 || !p95_ms.is_finite() || p95_ms <= 0.0 {
        return requested_fps;
    }
    let service_budget_ms = p95_ms * RTP_WRITER_HEADROOM_PERCENT / 100.0;
    let safe_fps = (1_000.0 / service_budget_ms).floor().max(1.0) as u32;
    safe_fps.clamp(
        requested_fps.min(INTERACTIVE_SERVICE_MIN_FPS),
        requested_fps,
    )
}

fn adaptive_bitrate_kbps(
    current_kbps: u32,
    target_kbps: u32,
    min_kbps: u32,
    dropped_delta: u64,
    queued_units: usize,
    in_flight_frames: usize,
    allow_increase: bool,
) -> u32 {
    let min_kbps = min_kbps.min(target_kbps).max(1);
    let pressure_ceiling = if dropped_delta > 0 || queued_units > 1 || in_flight_frames > 1 {
        current_kbps
            .saturating_mul(80)
            .saturating_div(100)
            .max(min_kbps)
    } else {
        current_kbps
    };
    if pressure_ceiling < current_kbps {
        return pressure_ceiling;
    }
    if allow_increase && current_kbps < target_kbps {
        return current_kbps
            .saturating_mul(105)
            .saturating_div(100)
            .min(target_kbps)
            .max(min_kbps);
    }
    current_kbps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_commits_only_after_encoder_accepts_proposal() {
        let mut controller = AdaptiveBitrateController::new(6_000);
        let proposed = controller
            .propose(1, 0, 0, 0, 0)
            .expect("drop pressure must propose a downshift");

        assert_eq!(proposed, 4_800);
        assert_eq!(controller.current_kbps, 6_000);
        controller.commit_applied(proposed);
        assert_eq!(controller.current_kbps, 4_800);
    }

    #[test]
    fn receiver_pressure_holds_recovery_before_bounded_upshift() {
        let mut controller = AdaptiveBitrateController::new(6_000);
        let proposed = controller
            .propose(0, 0, 0, 0, 1)
            .expect("receiver pressure must propose a downshift");
        controller.commit_applied(proposed);
        for _ in 0..7 {
            assert_eq!(controller.propose(0, 0, 0, 0, 0), None);
        }
        assert_eq!(controller.propose(0, 0, 0, 0, 0), Some(5_300));
    }

    #[test]
    fn receiver_pressure_accepts_only_fresh_monotonic_samples() {
        let mut tracker = ReceiverPressureTracker::default();
        let start = Instant::now();
        let baseline = ClientMediaFeedback {
            admission_sequence: 1,
            received_at: start,
            received_at_ms: 1_000,
            sampled_at_ms: 1_000,
            frames_dropped: 2,
            freeze_count: 1,
            jitter_buffer_avg_ms: 40.0,
            jitter_buffer_target_avg_ms: 40.0,
        };
        assert_eq!(
            tracker.observe(Some(baseline), start + Duration::from_millis(100)),
            ReceiverPressure::default()
        );
        let pressured = ClientMediaFeedback {
            admission_sequence: 2,
            received_at: start + Duration::from_secs(1),
            received_at_ms: 2_000,
            sampled_at_ms: 2_000,
            frames_dropped: 5,
            freeze_count: 3,
            jitter_buffer_avg_ms: 140.0,
            jitter_buffer_target_avg_ms: 60.0,
        };
        assert_eq!(
            tracker.observe(Some(pressured), start + Duration::from_millis(1_100)),
            ReceiverPressure {
                frames_dropped_delta: 3,
                freeze_delta: 2,
                elevated_jitter: true,
            }
        );
        assert_eq!(
            tracker.observe(Some(pressured), start + Duration::from_millis(1_200)),
            ReceiverPressure::default()
        );
        let stale = ClientMediaFeedback {
            admission_sequence: 3,
            received_at: start + Duration::from_secs(2),
            received_at_ms: 3_000,
            sampled_at_ms: 3_000,
            frames_dropped: 9,
            freeze_count: 4,
            jitter_buffer_avg_ms: 200.0,
            jitter_buffer_target_avg_ms: 200.0,
        };
        assert_eq!(
            tracker.observe(Some(stale), start + Duration::from_millis(12_001)),
            ReceiverPressure::default()
        );
    }

    #[test]
    fn effective_fps_tracks_applied_bitrate_with_interactive_floor() {
        assert_eq!(effective_fps_for_bitrate(60, 6_000, 6_000), 60);
        assert_eq!(effective_fps_for_bitrate(60, 4_800, 6_000), 48);
        assert_eq!(effective_fps_for_bitrate(60, 500, 6_000), 15);
        assert_eq!(effective_fps_for_bitrate(10, 500, 6_000), 10);
    }

    #[test]
    fn writer_service_time_independently_bounds_frame_rate() {
        assert_eq!(effective_fps_for_writer_service(30, 0.0, 0), 30);
        assert_eq!(effective_fps_for_writer_service(30, 10.0, 8), 30);
        assert_eq!(effective_fps_for_writer_service(30, 73.0, 1), 10);
        assert_eq!(effective_fps_for_writer_service(60, 500.0, 4), 5);
    }

    #[test]
    fn browser_wall_clock_cannot_poison_daemon_feedback_ordering() {
        let mut tracker = ReceiverPressureTracker::default();
        let start = Instant::now();
        let future_browser_clock = ClientMediaFeedback {
            admission_sequence: 1,
            received_at: start,
            received_at_ms: 1_000,
            sampled_at_ms: u64::MAX - 10,
            frames_dropped: 2,
            freeze_count: 0,
            jitter_buffer_avg_ms: 0.0,
            jitter_buffer_target_avg_ms: 0.0,
        };
        assert_eq!(
            tracker.observe(
                Some(future_browser_clock),
                start + Duration::from_millis(100)
            ),
            ReceiverPressure::default()
        );
        let normal_browser_clock = ClientMediaFeedback {
            admission_sequence: 2,
            received_at: start + Duration::from_secs(1),
            received_at_ms: 2_000,
            sampled_at_ms: 5,
            frames_dropped: 3,
            freeze_count: 0,
            jitter_buffer_avg_ms: 0.0,
            jitter_buffer_target_avg_ms: 0.0,
        };
        assert_eq!(
            tracker.observe(
                Some(normal_browser_clock),
                start + Duration::from_millis(1_100)
            ),
            ReceiverPressure {
                frames_dropped_delta: 1,
                ..ReceiverPressure::default()
            }
        );
    }

    #[test]
    fn admission_sequence_accepts_same_millisecond_and_clock_rollback() {
        let mut tracker = ReceiverPressureTracker::default();
        let start = Instant::now();
        let baseline = ClientMediaFeedback {
            admission_sequence: 1,
            received_at: start,
            received_at_ms: 2_000,
            sampled_at_ms: 1,
            frames_dropped: 2,
            freeze_count: 0,
            jitter_buffer_avg_ms: 0.0,
            jitter_buffer_target_avg_ms: 0.0,
        };
        assert_eq!(
            tracker.observe(Some(baseline), start),
            ReceiverPressure::default()
        );
        let same_millisecond = ClientMediaFeedback {
            admission_sequence: 2,
            received_at: start,
            received_at_ms: 2_000,
            frames_dropped: 3,
            ..baseline
        };
        assert_eq!(
            tracker.observe(Some(same_millisecond), start),
            ReceiverPressure {
                frames_dropped_delta: 1,
                ..ReceiverPressure::default()
            }
        );
        let clock_rollback = ClientMediaFeedback {
            admission_sequence: 3,
            received_at: start,
            received_at_ms: 1_900,
            frames_dropped: 4,
            ..same_millisecond
        };
        assert_eq!(
            tracker.observe(Some(clock_rollback), start),
            ReceiverPressure {
                frames_dropped_delta: 1,
                ..ReceiverPressure::default()
            }
        );
    }
}
