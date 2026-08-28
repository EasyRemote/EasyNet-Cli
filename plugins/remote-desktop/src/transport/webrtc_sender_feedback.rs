// EasyNet CLI — device-side WebRTC sender feedback
// =================================================
//
// File: plugins/remote-desktop/src/transport/webrtc_sender_feedback.rs
// Description: Fresh, direction-correct receiver pressure for one video sender.
//
// Protocol Responsibility:
// - None. RTCP is transport-local evidence and never changes Invocation authority.
//
// Implementation Approach:
// - Poll the generation-owned video RtpSender at a bounded cadence.
// - Consume only newer remote-inbound-rtp reports derived from RTCP RR.
// - Project one bounded pressure unit plus typed loss/RTT diagnostics.
//
// Usage Contract:
// - One tracker belongs to one transport epoch and one video sender.
// - Repeated stats snapshots never replay pressure; stats failures are non-terminal.
//
// Architectural Position:
// - RemoteDesktop plugin transport plane, immediately below shared media policy.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use rtc::statistics::report::RTCStatsReportEntry;
use webrtc::rtp_transceiver::RtpSender;

const RTCP_SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const RTCP_FRACTION_LOST_PRESSURE: f64 = 0.02;
const RTCP_ROUND_TRIP_PRESSURE_SECONDS: f64 = 0.250;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct RtcpReceiverPressure {
    pub(super) fresh: bool,
    pub(super) packets_lost_delta: u64,
    pub(super) fraction_lost: f64,
    pub(super) round_trip_time_ms: f64,
    pub(super) stats_read_failed: bool,
}

impl RtcpReceiverPressure {
    pub(super) fn pressure_units(self) -> u64 {
        u64::from(
            self.fresh
                && (self.packets_lost_delta > 0
                    || self.fraction_lost >= RTCP_FRACTION_LOST_PRESSURE
                    || self.round_trip_time_ms >= RTCP_ROUND_TRIP_PRESSURE_SECONDS * 1_000.0),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct RemoteInboundSample {
    measurements: u64,
    packets_lost: i64,
    fraction_lost: f64,
    round_trip_time_seconds: f64,
}

#[derive(Debug, Default)]
pub(super) struct RtcpReceiverPressureTracker {
    last_polled_at: Option<Instant>,
    last_measurements: u64,
    last_packets_lost: i64,
}

impl RtcpReceiverPressureTracker {
    pub(super) async fn observe(
        &mut self,
        sender: &Arc<dyn RtpSender>,
        observed_at: Instant,
    ) -> RtcpReceiverPressure {
        if self
            .last_polled_at
            .is_some_and(|last| observed_at.saturating_duration_since(last) < RTCP_SAMPLE_INTERVAL)
        {
            return RtcpReceiverPressure::default();
        }
        self.last_polled_at = Some(observed_at);
        let report = match sender.get_stats(observed_at).await {
            Ok(report) => report,
            Err(_) => {
                return RtcpReceiverPressure {
                    stats_read_failed: true,
                    ..RtcpReceiverPressure::default()
                };
            }
        };
        let sample = report
            .iter()
            .filter_map(|entry| match entry {
                RTCStatsReportEntry::RemoteInboundRtp(stats)
                    if stats.received_rtp_stream_stats.rtp_stream_stats.kind
                        == RtpCodecKind::Video =>
                {
                    Some(RemoteInboundSample {
                        measurements: stats.round_trip_time_measurements,
                        packets_lost: stats.received_rtp_stream_stats.packets_lost,
                        fraction_lost: stats.fraction_lost,
                        round_trip_time_seconds: stats.round_trip_time,
                    })
                }
                _ => None,
            })
            .max_by_key(|sample| sample.measurements);
        sample.map_or_else(RtcpReceiverPressure::default, |sample| {
            self.observe_sample(sample)
        })
    }

    fn observe_sample(&mut self, sample: RemoteInboundSample) -> RtcpReceiverPressure {
        if sample.measurements == 0 || sample.measurements <= self.last_measurements {
            return RtcpReceiverPressure::default();
        }
        let packets_lost_delta = sample
            .packets_lost
            .saturating_sub(self.last_packets_lost)
            .max(0) as u64;
        self.last_measurements = sample.measurements;
        self.last_packets_lost = self.last_packets_lost.max(sample.packets_lost);
        RtcpReceiverPressure {
            fresh: true,
            packets_lost_delta,
            fraction_lost: finite_non_negative(sample.fraction_lost),
            round_trip_time_ms: finite_non_negative(sample.round_trip_time_seconds) * 1_000.0,
            stats_read_failed: false,
        }
    }
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_reports_are_consumed_once_and_use_sender_direction_metrics() {
        let mut tracker = RtcpReceiverPressureTracker::default();
        let pressured = tracker.observe_sample(RemoteInboundSample {
            measurements: 1,
            packets_lost: 3,
            fraction_lost: 0.05,
            round_trip_time_seconds: 0.040,
        });
        assert!(pressured.fresh);
        assert_eq!(pressured.packets_lost_delta, 3);
        assert_eq!(pressured.pressure_units(), 1);

        assert_eq!(
            tracker.observe_sample(RemoteInboundSample {
                measurements: 1,
                packets_lost: 3,
                fraction_lost: 0.05,
                round_trip_time_seconds: 0.040,
            }),
            RtcpReceiverPressure::default()
        );
    }

    #[test]
    fn high_rtt_is_pressure_even_without_packet_loss() {
        let mut tracker = RtcpReceiverPressureTracker::default();
        let pressured = tracker.observe_sample(RemoteInboundSample {
            measurements: 2,
            packets_lost: 0,
            fraction_lost: 0.0,
            round_trip_time_seconds: 0.300,
        });
        assert_eq!(pressured.pressure_units(), 1);
        assert_eq!(pressured.round_trip_time_ms, 300.0);
    }

    #[test]
    fn invalid_metrics_are_fail_closed_to_zero_diagnostics() {
        let mut tracker = RtcpReceiverPressureTracker::default();
        let pressure = tracker.observe_sample(RemoteInboundSample {
            measurements: 1,
            packets_lost: -1,
            fraction_lost: f64::NAN,
            round_trip_time_seconds: -1.0,
        });
        assert_eq!(pressure.fraction_lost, 0.0);
        assert_eq!(pressure.round_trip_time_ms, 0.0);
        assert_eq!(pressure.pressure_units(), 0);
    }
}
